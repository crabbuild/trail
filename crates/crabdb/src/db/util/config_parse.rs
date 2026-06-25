use super::*;

pub(crate) fn parse_config_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(Error::InvalidInput(format!(
            "config key `{key}` expects a boolean value"
        ))),
    }
}

pub(crate) fn parse_config_suite_list(key: &str, value: &str) -> Result<Vec<String>> {
    let mut suites = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split([',', ';', '\n']) {
        let suite = raw.trim();
        if suite.is_empty() {
            continue;
        }
        if suite
            .chars()
            .any(|ch| matches!(ch, ',' | ';' | '\n' | '\r'))
        {
            return Err(Error::InvalidInput(format!(
                "config key `{key}` suite names cannot contain separators"
            )));
        }
        if seen.insert(suite.to_string()) {
            suites.push(suite.to_string());
        }
    }
    Ok(suites)
}

pub(crate) fn parse_config_u64(key: &str, value: &str, allow_zero: bool) -> Result<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("config key `{key}` expects an unsigned integer"))
    })?;
    if !allow_zero && parsed == 0 {
        return Err(Error::InvalidInput(format!(
            "config key `{key}` must be greater than zero"
        )));
    }
    Ok(parsed)
}
