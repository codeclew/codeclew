//! Streaming structural preflight for the new portable wire envelopes.
//! Leaf string bytes still obey the physical object bound. This is not a claim
//! that the subsequent logical bindings/fragment expansion has bounded RSS.
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

const MARKER: &str = "DOCS_BINDINGS_BUDGET";
struct State {
    nodes: usize,
}
struct Seed<'a> {
    state: &'a mut State,
    depth: usize,
    limit: usize,
    allowed: bool,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        if !self.allowed || self.depth > 128 || self.state.nodes >= 1_000_000 {
            return Err(serde::de::Error::custom(format!(
                "{MARKER}: portable JSON structure exceeds its bounds"
            )));
        }
        self.state.nodes += 1;
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded portable JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut count = 0;
        while seq
            .next_element_seed(Seed {
                state: self.state,
                depth: self.depth + 1,
                limit: 1_000_000,
                allowed: count < self.limit,
            })?
            .is_some()
        {
            count += 1;
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut count = 0;
        while let Some(key) = map.next_key::<String>()? {
            if count >= self.limit || key.len() > 65536 {
                return Err(serde::de::Error::custom(format!(
                    "{MARKER}: portable JSON membership exceeds its bound"
                )));
            }
            let limit = if self.depth == 0 {
                match key.as_str() {
                    "records" | "members" => 1024,
                    "objects" | "partitions" => 8192,
                    "tables" | "header" => 32,
                    _ => 1_000_000,
                }
            } else {
                1_000_000
            };
            map.next_value_seed(Seed {
                state: self.state,
                depth: self.depth + 1,
                limit,
                allowed: true,
            })?;
            count += 1;
        }
        Ok(())
    }
}

pub(super) fn preflight(raw: &[u8]) -> Result<(), crate::error::ClewError> {
    let mut parser = serde_json::Deserializer::from_slice(raw);
    let result = Seed {
        state: &mut State { nodes: 0 },
        depth: 0,
        limit: 32,
        allowed: true,
    }
    .deserialize(&mut parser)
    .and_then(|()| parser.end());
    result.map_err(|error| {
        if error.to_string().contains(MARKER) {
            super::budget("portable JSON node/depth/member/key bound exceeded")
        } else {
            super::corrupt("malformed portable JSON")
        }
    })
}
