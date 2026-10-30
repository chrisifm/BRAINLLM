use brain_backend::contract;
use serde_json::json;

#[test]
fn backend_selected_login_modes_preserve_custom_roles_and_closed_request_shapes() {
    let mut roles = brain_backend::auth_flows::RolesConfig::default();
    roles.roles.push("editor".into());
    let api = contract::openapi_with_roles(&[], &roles);
    let route = &api["paths"]["/_brain/auth/login"]["post"];
    let schemas = route["responses"]["200"]["content"]["application/json"]["schema"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(schemas.len(), 2);
    assert!(schemas.iter().any(|s| {
        s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "access_token")
    }));
    for schema in schemas {
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["user"]["properties"]["role"]["enum"],
            json!(roles.roles)
        );
    }
    let input = &route["requestBody"]["content"]["application/json"]["schema"];
    assert_eq!(input["additionalProperties"], false);
    assert!(input["properties"].get("role").is_none());
    assert_eq!(input["properties"]["password"]["writeOnly"], true);
}

#[test]
fn native_http_contract_keeps_success_statuses_and_binary_bodies() {
    let api = contract::openapi(&[]);
    let paths = &api["paths"];
    for (path, method, code) in [
        ("/_brain/links", "post", "201"),
        ("/_brain/links/{id}", "delete", "204"),
        ("/s/{code}", "get", "302"),
        ("/_brain/media/{id}", "delete", "202"),
        ("/_brain/auth/refresh", "post", "200"),
    ] {
        assert!(
            paths[path][method]["responses"][code].is_object(),
            "{method} {path}"
        );
    }
    assert!(
        paths["/_brain/links/{id}"]["delete"]["responses"]["204"]
            .get("content")
            .is_none()
    );
    assert!(paths["/s/{code}"]["get"]["responses"]["302"]["content"].is_null());
    assert_eq!(
        paths["/s/{code}"]["get"]["responses"]["302"]["headers"]["Location"]["required"],
        true
    );
    let image = &paths["/_brain/media/{id}/v/{version}/{preset}"];
    assert!(image["get"]["responses"]["200"]["content"]["image/webp"].is_object());
    assert!(image["head"]["responses"]["200"].get("content").is_none());
    for name in ["id", "version", "preset"] {
        assert!(
            image["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == name && p["required"] == true && p["in"] == "path")
        );
    }
}

#[test]
fn native_auth_and_mutations_declare_the_real_transport_contract() {
    let api = contract::openapi(&[]);
    let paths = &api["paths"];
    assert_eq!(
        paths["/_brain/auth/refresh"]["post"]["security"],
        json!([{"refreshCookie":[]}])
    );
    assert_eq!(
        paths["/_brain/auth/google"]["post"]["security"],
        json!([{"googleNonce":[]}])
    );
    assert_eq!(
        api["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    assert_eq!(
        api["components"]["securitySchemes"]["accessCookie"]["name"],
        "__Host-brain-access"
    );
    for path in [
        "/_brain/auth/token",
        "/_brain/media/{id}",
        "/_brain/jobs/{queue}",
    ] {
        let method = if path == "/_brain/media/{id}" {
            "patch"
        } else {
            "post"
        };
        assert!(
            paths[path][method]["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "Origin" && p["required"] == true)
        );
    }
    assert_eq!(
        paths["/_brain/jobs/{queue}"]["post"]["x-brain-required-role"],
        "admin"
    );
    assert_eq!(
        paths["/_brain/jobs/status/{id}"]["get"]["x-brain-required-role"],
        "admin"
    );
    let schema = &paths["/_brain/media/{id}/variants"]["post"]["requestBody"]["content"]["application/json"]
        ["schema"];
    assert_eq!(schema["properties"]["presets"]["maxItems"], 6);
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn webhook_contract_distinguishes_signed_providers_from_admin_delivery_and_never_returns_payloads()
{
    let api = contract::openapi(&[]);
    let inbound = &api["paths"]["/_brain/webhooks/in/{name}"]["post"];
    assert_eq!(
        inbound["x-brain-signature"]["timestampToleranceSeconds"],
        300
    );
    assert_eq!(inbound["x-brain-signature"]["originRequired"], false);
    assert_eq!(inbound["security"], json!([]));
    let headers = inbound["parameters"].as_array().unwrap();
    assert_eq!(headers.len(), 3);
    assert!(
        headers
            .iter()
            .all(|header| header["in"] == "header" && header["required"] == true)
    );
    let outbound = &api["paths"]["/_brain/webhooks/out/{name}"]["post"];
    assert!(!outbound["security"].as_array().unwrap().is_empty());
    assert_eq!(
        outbound["requestBody"]["content"]["application/json"]["schema"]["additionalProperties"],
        false
    );
    let receipt = &outbound["responses"]["202"]["content"]["application/json"]["schema"];
    assert_eq!(receipt["additionalProperties"], false);
    assert!(receipt["properties"].get("payload").is_none());
    let status = &api["paths"]["/_brain/webhooks/out/{name}/{id}"]["get"]["responses"]["200"]["content"]
        ["application/json"]["schema"];
    assert_eq!(status["properties"]["deliveries"]["maxItems"], 5);
    for field in ["payload", "url", "headers", "secret"] {
        assert!(status["properties"].get(field).is_none());
    }
}
