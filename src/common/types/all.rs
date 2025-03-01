
use log::debug;

use serde::{Deserialize, Serialize};

use num_bigint::{BigInt, ToBigInt};

use clvmr::allocator::{NodePtr, SExp};
use clvmr::serde::node_to_bytes;
use clvmr::Allocator;
use clvmr::{run_program, ChiaDialect, NO_UNKNOWN_OPS};

use crate::utils::proper_list;

use crate::common::constants::{AGG_SIG_ME_ATOM, AGG_SIG_UNSAFE_ATOM, CREATE_COIN_ATOM, REM_ATOM};

use clvm_traits::{ClvmEncoder, ToClvm, ToClvmError};

use crate::common::types::coin_id::{AllocEncoder, Hash};
use crate::common::types::coin_string::{u64_from_atom, Amount, CoinString, PuzzleHash};
use crate::common::types::error::{Error, IntoErr};
use crate::common::types::private_key::{Aggsig, PublicKey};
use crate::common::types::program::{Program, ProgramRef, Puzzle};

pub fn chia_dialect() -> ChiaDialect {
    ChiaDialect::new(NO_UNKNOWN_OPS)
}

#[derive(Clone, Debug)]
pub struct Node(pub NodePtr);

impl Default for Node {
    fn default() -> Node {
        let allocator = Allocator::new();
        Node(allocator.nil())
    }
}

impl Node {
    pub fn to_hex(&self, allocator: &mut AllocEncoder) -> Result<String, Error> {
        let bytes = node_to_bytes(allocator.allocator(), self.0).into_gen()?;
        Ok(hex::encode(bytes))
    }
}

impl<E: ClvmEncoder<Node = NodePtr>> ToClvm<E> for Node {
    fn to_clvm(&self, _encoder: &mut E) -> Result<<E as ClvmEncoder>::Node, ToClvmError> {
        Ok(self.0)
    }
}

#[derive(Debug, Clone)]
pub enum CoinCondition {
    AggSigMe(PublicKey, Vec<u8>),
    AggSigUnsafe(PublicKey, Vec<u8>),
    #[allow(dead_code)]
    CreateCoin(PuzzleHash, Amount),
    Rem(Vec<Vec<u8>>),
}

fn parse_condition(allocator: &mut AllocEncoder, condition: NodePtr) -> Option<CoinCondition> {
    let exploded = proper_list(allocator.allocator(), condition, true)?;
    let public_key_from_bytes = |b: &[u8]| -> Result<PublicKey, Error> {
        let mut fixed: [u8; 48] = [0; 48];
        for (i, b) in b.iter().enumerate() {
            fixed[i % 48] = *b;
        }
        PublicKey::from_bytes(fixed)
    };
    if exploded.len() > 2
        && matches!(
            (
                allocator.allocator().sexp(exploded[0]),
                allocator.allocator().sexp(exploded[1]),
                allocator.allocator().sexp(exploded[2])
            ),
            (SExp::Atom, SExp::Atom, SExp::Atom)
        )
    {
        let atoms: Vec<Vec<u8>> = exploded
            .iter()
            .take(3)
            .map(|a| allocator.allocator().atom(*a).to_vec())
            .collect();
        if *atoms[0] == AGG_SIG_UNSAFE_ATOM {
            if let Ok(pk) = public_key_from_bytes(&atoms[1]) {
                return Some(CoinCondition::AggSigUnsafe(pk, atoms[2].to_vec()));
            }
        } else if *atoms[0] == AGG_SIG_ME_ATOM {
            if let Ok(pk) = public_key_from_bytes(&atoms[1]) {
                return Some(CoinCondition::AggSigMe(pk, atoms[2].to_vec()));
            }
        } else if *atoms[0] == CREATE_COIN_ATOM {
            if let Some(amt) = u64_from_atom(&atoms[2]) {
                return Some(CoinCondition::CreateCoin(
                    PuzzleHash::from_hash(Hash::from_slice(&atoms[1])),
                    Amount::new(amt),
                ));
            }
        }
    }

    if !exploded.is_empty()
        && exploded
            .iter()
            .all(|e| matches!(allocator.allocator().sexp(*e), SExp::Atom))
    {
        let atoms: Vec<Vec<u8>> = exploded
            .iter()
            .map(|a| allocator.allocator().atom(*a).to_vec())
            .collect();
        if *atoms[0] == REM_ATOM {
            return Some(CoinCondition::Rem(
                atoms.iter().skip(1).map(|a| a.to_vec()).collect(),
            ));
        }
    }

    None
}

impl CoinCondition {
    pub fn from_nodeptr(allocator: &mut AllocEncoder, conditions: NodePtr) -> Vec<CoinCondition> {
        // Ensure this borrow of allocator is finished for what's next.
        if let Some(exploded) = proper_list(allocator.allocator(), conditions, true) {
            exploded
                .iter()
                .flat_map(|cond| parse_condition(allocator, *cond))
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn from_puzzle_and_solution(
        allocator: &mut AllocEncoder,
        puzzle: &Program,
        solution: &Program,
    ) -> Result<Vec<CoinCondition>, Error> {
        let run_puzzle = puzzle.to_nodeptr(allocator)?;
        let run_args = solution.to_nodeptr(allocator)?;
        let conditions = run_program(
            allocator.allocator(),
            &chia_dialect(),
            run_puzzle,
            run_args,
            0,
        )
        .into_gen()?;
        debug!(
            "conditions to parse {}",
            Node(conditions.1).to_hex(allocator)?
        );

        Ok(CoinCondition::from_nodeptr(allocator, conditions.1))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spend {
    pub puzzle: Puzzle,
    pub solution: ProgramRef,
    pub signature: Aggsig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoinSpend {
    pub coin: CoinString,
    pub bundle: Spend,
}

impl Default for Spend {
    fn default() -> Self {
        Spend {
            puzzle: Puzzle::from_bytes(&[0x80]),
            solution: Program::from_bytes(&[0x80]).into(),
            signature: Aggsig::default(),
        }
    }
}

pub struct SpendRewardResult {
    pub coins_with_solutions: Vec<CoinSpend>,
    pub result_coin_string_up: CoinString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendBundle {
    pub name: Option<String>,
    pub spends: Vec<CoinSpend>,
}

/// Maximum information about a coin spend.  Everything one might need downstream.
pub struct BrokenOutCoinSpendInfo {
    pub solution: ProgramRef,
    pub conditions: ProgramRef,
    pub message: Vec<u8>,
    pub signature: Aggsig,
}

pub fn divmod(a: BigInt, b: BigInt) -> (BigInt, BigInt) {
    let d = a.clone() / b.clone();
    let r = a.clone() % b.clone();
    let zero = 0.to_bigint().unwrap();
    if d < zero && r != zero {
        (d - 1.to_bigint().unwrap(), r + b)
    } else {
        (d, r)
    }
}

#[test]
fn test_local_divmod() {
    assert_eq!(
        divmod((-7).to_bigint().unwrap(), 2.to_bigint().unwrap()),
        ((-4).to_bigint().unwrap(), 1.to_bigint().unwrap())
    );
    assert_eq!(
        divmod(7.to_bigint().unwrap(), (-2).to_bigint().unwrap()),
        ((-4).to_bigint().unwrap(), (-1).to_bigint().unwrap())
    );
    assert_eq!(
        divmod((-7).to_bigint().unwrap(), (-2).to_bigint().unwrap()),
        (3.to_bigint().unwrap(), (-1).to_bigint().unwrap())
    );
    assert_eq!(
        divmod(7.to_bigint().unwrap(), 2.to_bigint().unwrap()),
        (3.to_bigint().unwrap(), 1.to_bigint().unwrap())
    );
}
