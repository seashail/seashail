use serde_json::Value;

pub(super) fn parse_usd_value(args: &Value) -> (f64, bool) {
    super::super::value_helpers::parse_usd_value(args)
}

pub(super) fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

pub(super) fn get_asset_obj(args: &Value) -> Option<&Value> {
    super::super::value_helpers::get_asset_obj(args)
}

pub(super) fn get_str_in_args_or_asset<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    if let Some(v) = get_str(args, key) {
        return Some(v);
    }
    super::super::value_helpers::get_str_in_args_or_asset(args, key)
}

pub(super) fn summarize_sim_error(e: &eyre::Report, label: &str) -> String {
    super::super::value_helpers::summarize_sim_error(e, label)
}
