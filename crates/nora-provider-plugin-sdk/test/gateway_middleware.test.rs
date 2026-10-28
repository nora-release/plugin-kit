use super::*;
#[test]
fn rejects_only_before_routing_and_drops_only_events() {
    let reject = json!({"version":1,"action":"reject","reject":{"status":429,"message":"No"}});
    assert_eq!(
        reply(Phase::Request, reject.clone()).unwrap().action,
        Action::Reject(Rejection {
            status: 429,
            message: "No".into()
        })
    );
    for phase in [Phase::Provider, Phase::Event, Phase::Response] {
        assert_eq!(reply(phase, reject.clone()).unwrap().action, Action::Keep);
    }
    let drop = json!({"version":1,"action":"drop"});
    assert_eq!(
        reply(Phase::Event, drop.clone()).unwrap().action,
        Action::Drop
    );
    assert_eq!(reply(Phase::Request, drop).unwrap().action, Action::Keep);
}
#[test]
fn replacements_allow_model_and_tools_without_relaxing_gateway_hooks() {
    let before = json!({"model":"a","tools":[]});
    let after = json!({"model":"b","tools":[{"name":"x"}]});
    assert!(crate::gateway_hooks::validate_change(&before, &after).is_err());
    assert_eq!(
        reply(
            Phase::Request,
            json!({"version":1,"action":"replace","body":after})
        )
        .unwrap()
        .action,
        Action::Replace(after)
    );
}
#[test]
fn invalid_states_and_results_cannot_enter_a_chain() {
    for result in [
        json!({"version":2,"action":"keep"}),
        json!({"version":1,"action":"replace","body":null}),
        json!({"version":1,"action":"keep","state":[]}),
        json!({"version":1,"action":"keep","state":{"large":"x".repeat(MAX_STATE)}}),
    ] {
        assert!(reply(Phase::Request, result).is_err());
    }
    let action = reply(
        Phase::Request,
        json!({"version":1,"action":"reject","reject":{"status":200}}),
    )
    .unwrap()
    .action;
    assert_eq!(
        action,
        Action::Reject(Rejection {
            status: 403,
            message: "turned away by a gateway middleware".into()
        })
    );
}
#[test]
fn every_strategy_must_return_exactly_a_permutation() {
    assert_eq!(
        order(&json!({"version":1,"order":[2,0,1]}), 3),
        Some(vec![2, 0, 1])
    );
    for invalid in [
        json!([0, 0, 1]),
        json!([0, 1]),
        json!([0, 1, 3]),
        json!([-1, 0, 1]),
        json!([0.0, 1, 2]),
        json!(["0", 1, 2]),
    ] {
        assert_eq!(order(&json!({"version":1,"order":invalid}), 3), None);
    }
}
#[test]
fn declaration_and_phase_event_filter_are_validated() {
    let d:Declaration=serde_json::from_value(json!({"version":1,"event":true,"events":["message_start"],"strategies":["accounts-first"]})).unwrap();
    d.validate().unwrap();
    assert!(d.participates(Phase::Event, Some("message_start")));
    assert!(!d.participates(Phase::Event, Some("text_delta")));
    assert!(!d.participates(Phase::Event, None));
    assert_eq!(
        strategy_id("reserve/accounts-first"),
        Some(("reserve", "accounts-first"))
    );
    for id in ["/x", "foo/../bad", "foo/x/y", "foo/ x", "foo/-x"] {
        assert_eq!(strategy_id(id), None);
    }
}
