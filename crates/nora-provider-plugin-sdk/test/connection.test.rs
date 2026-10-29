use super::*;

#[test]
fn additional_apis_keep_the_primary_and_independent_default_addresses() {
    let defaults = [
        ("chat", "https://chat.example/v1"),
        ("responses", "https://responses.example/v1"),
    ];
    let endpoints = api_endpoints(
        "chat",
        "https://chat.example/v1",
        "https://chat.example/v1/",
        defaults,
        [("messages", "https://messages.example")],
    );
    assert_eq!(endpoints[0].0, "chat");
    let endpoints = endpoints.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(endpoints.len(), 3);
    assert_eq!(endpoints["chat"], "https://chat.example/v1");
    assert_eq!(endpoints["responses"], "https://responses.example/v1");
    assert_eq!(endpoints["messages"], "https://messages.example");
}

#[test]
fn explicit_addresses_win_and_a_moved_primary_does_not_keep_vendor_addresses() {
    let endpoints = api_endpoints(
        "chat",
        "https://private.example/v1",
        "https://vendor.example/v1",
        [("responses", "https://vendor.example/responses")],
        [("chat", "https://explicit.example/v1")],
    );
    assert_eq!(endpoints.len(), 1);
    assert_eq!(
        endpoints[0],
        ("chat".into(), "https://explicit.example/v1".into())
    );
}
