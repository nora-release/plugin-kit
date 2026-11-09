use super::*;
use serde_json::json;

#[test]
fn the_wire_version_and_tool_schema_are_language_neutral() {
    let params = InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        plugin_id: "sample".into(),
        config: json!({"enabled": true}),
        granted_services: vec!["session.store".into()],
        stream_window: None,
    };
    let request = Request::new(1, INITIALIZE, serde_json::to_value(params).unwrap());
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["params"]["protocolVersion"], 1);
    assert_eq!(value["params"]["grantedServices"], json!(["session.store"]));
}

#[test]
fn response_validation_rejects_ambiguous_results_and_wrong_versions() {
    let mut response = Response::success(2, json!({"ok": true}));
    assert!(response.is_valid());
    response.error = Some(RpcError::method_not_found());
    assert!(!response.is_valid());
    response.error = None;
    response.jsonrpc = "1.0".into();
    assert!(!response.is_valid());
}

#[test]
fn a_null_success_is_distinct_from_a_missing_result() {
    let response: Response =
        serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":null})).unwrap();
    assert!(response.is_valid());
    assert_eq!(response.result, Some(Value::Null));
    let missing: Response = serde_json::from_value(json!({"jsonrpc":"2.0","id":1})).unwrap();
    assert!(!missing.is_valid());
}

#[test]
fn tool_results_round_trip_without_agent_types() {
    let result = ToolResult {
        content: vec![
            ContentBlock::Text {
                text: "hello".into(),
            },
            ContentBlock::Image {
                data: "aW1hZ2U=".into(),
                mime_type: "image/png".into(),
            },
        ],
        is_error: false,
        details: Some(json!({"count": 2})),
    };
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["content"][1]["mimeType"], "image/png");
    assert_eq!(serde_json::from_value::<ToolResult>(value).unwrap(), result);
}

#[test]
fn older_initialization_results_default_to_no_named_services() {
    let result: InitializeResult =
        serde_json::from_value(json!({"protocolVersion":1,"tools":[]})).unwrap();
    assert!(result.services.is_empty());
    let result: InitializeResult =
        serde_json::from_value(json!({"protocolVersion":1,"services":["llm","providers"]}))
            .unwrap();
    assert_eq!(result.services, ["llm", "providers"]);
    assert!(valid_service_name(&result.services[0]));
    assert!(!valid_service_name("llm/providers"));
}

#[test]
fn version_one_frames_carry_no_stream_fields() {
    let params = InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        plugin_id: "sample".into(),
        config: Value::Null,
        granted_services: Vec::new(),
        stream_window: None,
    };
    let value = serde_json::to_value(params).unwrap();
    assert!(value.get("streamWindow").is_none());
    let call = serde_json::to_value(ServiceCallParams {
        service: "s".into(),
        method: "m".into(),
        arguments: Value::Null,
        parent_request_id: None,
        input_stream: None,
        output_stream: None,
    })
    .unwrap();
    assert_eq!(
        call,
        json!({"service": "s", "method": "m", "arguments": null})
    );
}

#[test]
fn stream_frames_name_their_stream_and_cost_a_fixed_overhead() {
    let data = StreamDataParams {
        stream: 3,
        data: "aGk=".into(),
        end: false,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&data).unwrap(),
        json!({"stream": 3, "data": "aGk="})
    );
    let ended: StreamDataParams = serde_json::from_value(
        json!({"stream": 4, "end": true, "error": {"code": 1, "message": "x"}}),
    )
    .unwrap();
    assert!(ended.end && ended.data.is_empty() && ended.error.is_some());
    assert!(serde_json::from_value::<StreamCreditParams>(json!({"stream": 1})).is_err());
    assert_eq!(stream_frame_cost(0), STREAM_FRAME_COST);
    assert!(supported_protocol(1) && supported_protocol(2) && !supported_protocol(3));
}
