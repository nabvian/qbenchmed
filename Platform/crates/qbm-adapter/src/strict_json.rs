use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, de};
use serde_json::{Map, Number, Value};

pub(super) fn parse(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> de::Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(1_024));
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some(key) = object.next_key::<String>()? {
            if entries.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate object key {key:?}")));
            }
            let value = object.next_value::<StrictValue>()?;
            entries.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(
            entries.into_iter().collect::<Map<_, _>>(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_nested_duplicate_keys() {
        let error = parse(br#"{"outer":{"same":1,"same":2}}"#).unwrap_err();
        assert!(error.to_string().contains("duplicate object key \"same\""));
    }

    #[test]
    fn preserves_all_json_value_categories() {
        let value = parse(br#"{"a":[null,true,-2,3,4.5,"x"],"b":{}}"#).unwrap();
        assert_eq!(value, json!({"a": [null, true, -2, 3, 4.5, "x"], "b": {}}));
    }
}
