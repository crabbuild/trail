use serde::de::Deserialize;
use serde_json::Value;

use crate::{Error, Result};

pub(crate) fn from_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T> {
    if arguments.is_null() {
        serde_json::from_value(serde_json::json!({})).map_err(Error::from)
    } else {
        serde_json::from_value(arguments).map_err(Error::from)
    }
}
