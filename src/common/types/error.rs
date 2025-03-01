use clvm_traits::ToClvmError;
use clvmr::reduction::EvalErr;
use serde::{Deserialize, Serialize, Serializer};
use std::io;

/// Error type
#[derive(Debug)]
pub enum Error {
    ClvmErr(EvalErr),
    IoErr(io::Error),
    BasicErr,
    EncodeErr(ToClvmError),
    StrErr(String),
    BlsErr(chia_bls::Error),
    BsonErr(bson::de::Error),
    JsonErr(serde_json::Error),
    HexErr(hex::FromHexError),
    Channel(String),
    GameMoveRejected(Vec<u8>),
}

#[derive(Serialize, Deserialize)]
struct SerializedError {
    error: String,
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerializedError {
            error: format!("{self:?}"),
        }
        .serialize(serializer)
    }
}
