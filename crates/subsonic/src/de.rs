// SPDX-License-Identifier: GPL-3.0-only
//! Lenient deserializers for identifiers: servers send them as strings or numbers.

use serde::{Deserialize, Deserializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum StrOrNum {
    Str(String),
    Int(i64),
    UInt(u64),
    Float(f64),
}

impl From<StrOrNum> for String {
    fn from(v: StrOrNum) -> Self {
        match v {
            StrOrNum::Str(s) => s,
            StrOrNum::Int(i) => i.to_string(),
            StrOrNum::UInt(u) => u.to_string(),
            StrOrNum::Float(f) => f.to_string(),
        }
    }
}

/// Deserialize a mandatory id that may be a string or a number.
pub(crate) fn id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    StrOrNum::deserialize(d).map(Into::into)
}

/// Deserialize an optional id that may be a string, a number or `null`.
pub(crate) fn opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<StrOrNum>::deserialize(d)?
        .map(String::from)
        .filter(|s| !s.is_empty()))
}
