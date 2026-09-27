//! Stable Desktop connection identity; never a credential or a process PID.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TunnelProfileId(uuid::Uuid);

impl TunnelProfileId {
    /// Reserved for the migrated singleton and explicit legacy environment fallback.
    pub const DEFAULT: Self = Self(uuid::Uuid::nil());

    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl TryFrom<String> for TunnelProfileId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "default" {
            return Ok(Self::DEFAULT);
        }
        let id = uuid::Uuid::parse_str(&value).map_err(|_| "Invalid connection identity")?;
        if id.is_nil() || id.hyphenated().to_string() != value {
            return Err("Invalid connection identity");
        }
        Ok(Self(id))
    }
}

impl From<TunnelProfileId> for String {
    fn from(id: TunnelProfileId) -> Self {
        if id == TunnelProfileId::DEFAULT {
            "default".to_string()
        } else {
            id.0.hyphenated().to_string()
        }
    }
}

impl fmt::Display for TunnelProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&String::from(*self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_roundtrip_and_reject_path_like_input() {
        for id in [
            TunnelProfileId::DEFAULT,
            TunnelProfileId::new(),
            TunnelProfileId::new(),
        ] {
            let value = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<TunnelProfileId>(&value).unwrap(), id);
        }
        assert_eq!(TunnelProfileId::DEFAULT.to_string(), "default");
        for invalid in [
            "",
            "../profile",
            "personal/key",
            "00000000-0000-0000-0000-000000000000",
        ] {
            assert!(TunnelProfileId::try_from(invalid.to_string()).is_err());
        }
    }
}
