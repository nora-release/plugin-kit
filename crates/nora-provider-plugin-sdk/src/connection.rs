use std::collections::BTreeMap;

/// Additional API addresses override matching defaults without removing the
/// primary API, which stays first for model discovery. Moving the primary
/// address drops the old vendor's defaults.
pub fn api_endpoints<'a>(
    primary_api: &str,
    primary_base: &str,
    default_base: &str,
    defaults: impl IntoIterator<Item = (&'a str, &'a str)>,
    overrides: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<(String, String)> {
    let mut endpoints = BTreeMap::<String, String>::new();
    if primary_base.trim_end_matches('/') == default_base.trim_end_matches('/') {
        endpoints.extend(
            defaults
                .into_iter()
                .map(|(api, base)| (api.into(), base.into())),
        );
    }
    if !primary_api.is_empty() && !primary_base.is_empty() {
        endpoints.insert(primary_api.into(), primary_base.into());
    }
    endpoints.extend(
        overrides
            .into_iter()
            .map(|(api, base)| (api.into(), base.into())),
    );
    let primary = endpoints.remove_entry(primary_api);
    primary.into_iter().chain(endpoints).collect()
}

#[cfg(test)]
#[path = "../test/connection.test.rs"]
mod tests;
