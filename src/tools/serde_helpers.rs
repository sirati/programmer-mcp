//! Serde helpers shared across operation types.

pub fn default_max_depth() -> usize {
    3
}

pub fn default_trigger_lines_after() -> usize {
    5
}

pub fn default_trigger_timeout() -> u64 {
    3000
}

pub fn default_context_lines() -> usize {
    5
}

pub fn default_true() -> bool {
    true
}

/// Deserialize either a single string or a vec of strings into `Vec<String>`.
pub fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrVec;

    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or array of strings")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(vec![v.to_string()])
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(vec![v])
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut vec = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                vec.push(s);
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}
