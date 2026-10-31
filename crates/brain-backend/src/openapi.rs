//! OpenAPI entries for application routes and file buckets, merged into the resource document.
use crate::files::{Bucket, Visibility};
use crate::routes::{Access, Kind, Route};
use serde_json::{Map, Value, json};

fn security(access: &Access) -> Value {
    match access {
        Access::Public => json!([]),
        _ => json!([{"session":[]},{"accessCookie":[]},{"bearer":[]}]),
    }
}
fn kind(kind: Kind) -> Value {
    match kind {
        Kind::Str => json!({"type":"string"}),
        Kind::Int => json!({"type":"integer"}),
        Kind::Bool => json!({"type":"boolean"}),
        Kind::Uuid => json!({"type":"string","format":"uuid"}),
    }
}
fn error(description: &str) -> Value {
    json!({"description":description,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/BrainError"}}}})
}

fn route_operation(route: &Route) -> Value {
    let mut parameters: Vec<Value> = route.params.iter().map(|(name, k)| json!({"name":name,"in":"path","required":true,"schema":kind(*k)})).collect();
    parameters.extend(route.query.iter().map(|(name, q)| json!({"name":name,"in":"query","required":q.required,"schema":kind(q.kind)})));
    parameters.extend(route.headers.iter().map(|name| json!({"name":name,"in":"header","required":false,"schema":{"type":"string"}})));
    if let Some(verify) = &route.verify {
        parameters.push(json!({"name":verify.header,"in":"header","required":true,"schema":{"type":"string"},"description":"HMAC-SHA256 signature of the raw body"}));
    }
    let mut responses = json!({
        "200": {"description":"Handler result (status and headers chosen by the handler)","content":{"application/json":{"schema":{}}}},
        "400": error("Invalid JSON body"), "404": error("Unknown path or invalid path parameter"),
        "413": error("Body above the route limit"), "422": error("Invalid query or handler field errors"),
        "500": error("Handler failed"), "503": error("Route executor at capacity"), "504": error("Handler timed out"),
    });
    if !matches!(route.access, Access::Public) {
        responses["401"] = error("Sign in required");
    }
    if matches!(route.access, Access::Roles(_)) {
        responses["403"] = error("Role required");
    }
    if route.rate.is_some() {
        responses["429"] = error("Rate limited; see Retry-After");
    }
    if route.verify.is_some() {
        responses["401"] = error("Signature missing, invalid or expired");
    }
    let mut op = json!({
        "operationId": format!("route_{}", route.name),
        "security": security(&route.access),
        "parameters": parameters,
        "responses": responses,
        "x-brain-route": route.schema(),
    });
    if !matches!(route.method.as_str(), "GET" | "DELETE") {
        let content = if route.raw_body {
            json!({"*/*":{"schema":{"type":"string","format":"binary","maxLength":route.body_limit}}})
        } else {
            json!({"application/json":{"schema":{"type":"object"}}})
        };
        op["requestBody"] = json!({"required": route.verify.is_some(), "content": content});
    }
    op
}

fn bucket_paths(bucket: &Bucket, paths: &mut Map<String, Value>) {
    let file = json!({"$ref":"#/components/schemas/BrainFile"});
    let id = json!({"name":"id","in":"path","required":true,"schema":{"type":"string","format":"uuid"}});
    let mut upload_params = vec![];
    if bucket.resource.is_some() {
        upload_params.push(json!({"name":"record","in":"query","required":true,"schema":{"type":"string","format":"uuid"}}));
    }
    let types = if bucket.types.is_empty() { json!("any") } else { json!(bucket.types) };
    let public = bucket.visibility == Visibility::Public;
    paths.insert(format!("/_brain/files/{}", bucket.name), json!({"post": {
        "operationId": format!("files_{}_upload", bucket.name),
        "security": security(&Access::SignedIn),
        "parameters": upload_params,
        "requestBody": {"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["file"],"properties":{"file":{"type":"string","format":"binary"}}}}}},
        "responses": {"201":{"description":"Stored","content":{"application/json":{"schema":file}}},"401":error("Sign in required"),"403":error("Policy denied"),"413":error("File above maxBytes"),"415":error("Content type not allowed")},
        "x-brain-bucket": {"maxBytes":bucket.max_bytes,"types":types,"visibility":bucket.visibility.as_str(),"create":bucket.create,"read":bucket.read,"delete":bucket.delete},
    }}));
    paths.insert(format!("/_brain/files/{}/{{id}}", bucket.name), json!({
        "get": {"operationId":format!("files_{}_download",bucket.name),"security":if public { json!([]) } else { security(&Access::SignedIn) },"parameters":[id,{"name":"Range","in":"header","required":false,"schema":{"type":"string"}}],
            "responses":{"200":{"description":"File bytes","content":{"*/*":{"schema":{"type":"string","format":"binary"}}}},"206":{"description":"Partial content"},"304":{"description":"Not modified"},"404":error("Not found or not readable"),"416":error("Range not satisfiable")}},
        "delete": {"operationId":format!("files_{}_delete",bucket.name),"security":security(&Access::SignedIn),"parameters":[id],
            "responses":{"200":{"description":"Deleted","content":{"application/json":{"schema":{"type":"object","properties":{"ok":{"type":"boolean"}}}}}},"404":error("Not found or not deletable")}},
    }));
    paths.insert(format!("/_brain/files/{}/{{id}}/meta", bucket.name), json!({"get": {
        "operationId": format!("files_{}_meta", bucket.name), "security": security(&Access::SignedIn), "parameters": [id],
        "responses": {"200":{"description":"Metadata","content":{"application/json":{"schema":file}}},"404":error("Not found or not readable")},
    }}));
}

/// Adds route and bucket operations plus their shared schemas to an OpenAPI document.
pub fn extend(doc: &mut Value, routes: &[Route], buckets: &[Bucket]) {
    if routes.is_empty() && buckets.is_empty() {
        return;
    }
    let mut paths = doc.get("paths").and_then(Value::as_object).cloned().unwrap_or_default();
    for route in routes {
        let entry = paths.entry(route.path.clone()).or_insert_with(|| json!({}));
        entry[route.method.as_str().to_ascii_lowercase()] = route_operation(route);
    }
    for bucket in buckets {
        bucket_paths(bucket, &mut paths);
    }
    doc["paths"] = Value::Object(paths);
    if !doc["components"].is_object() {
        doc["components"] = json!({});
    }
    if !doc["components"]["schemas"].is_object() {
        doc["components"]["schemas"] = json!({});
    }
    doc["components"]["schemas"]["BrainError"] = json!({"type":"object","required":["error"],"properties":{"schemaVersion":{"type":"integer"},"rid":{"type":"string"},"error":{"type":"object","required":["code"],"properties":{"code":{"type":"string"},"fix":{"type":"string"},"fields":{}}}}});
    if !buckets.is_empty() {
        doc["components"]["schemas"]["BrainFile"] = json!({"type":"object","properties":{"id":{"type":"string","format":"uuid"},"bucket":{"type":"string"},"filename":{"type":"string"},"contentType":{"type":"string"},"bytes":{"type":"integer"},"sha256":{"type":"string"},"visibility":{"type":"string","enum":["private","public"]},"createdAt":{"type":"string","format":"date-time"},"url":{"type":"string"}}});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_buckets_become_documented_operations() {
        let env = |n: &str| (n == "SHOP_HOOK_SECRET").then(|| "synthetic-shop-hook-secret-0001".to_string());
        let quote = Route::from_contract("order_quote", &json!({"method":"POST","path":"/orders/{id}/quote","params":{"id":"uuid"},"query":{"currency":"str!"},"headers":["x-shop-channel"],"rate":"5/min per user"}), &env).unwrap();
        let hook = Route::from_contract("shop_events", &json!({"method":"POST","path":"/shop/events","auth":"public","body":"raw","verify":{"header":"x-shop-signature","secret":"SHOP_HOOK_SECRET","prefix":"sha256="}}), &env).unwrap();
        let bucket = Bucket::from_contract("invoices", json!({"types":["application/pdf"],"maxBytes":"10mb"})).unwrap();
        let mut doc = json!({"openapi":"3.1.0","paths":{"/api/products":{"get":{}}}});
        extend(&mut doc, &[quote, hook], &[bucket]);
        let op = &doc["paths"]["/orders/{id}/quote"]["post"];
        assert_eq!(op["operationId"], "route_order_quote");
        assert_eq!(op["parameters"][0], json!({"name":"id","in":"path","required":true,"schema":{"type":"string","format":"uuid"}}));
        assert_eq!(op["parameters"][1]["required"], true);
        assert!(op["responses"]["429"].is_object() && op["responses"]["401"].is_object());
        let hook = &doc["paths"]["/shop/events"]["post"];
        assert_eq!(hook["security"], json!([]));
        assert!(hook["requestBody"]["content"]["*/*"].is_object());
        assert!(hook["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "x-shop-signature"));
        assert!(!doc.to_string().contains("synthetic-shop-hook-secret"), "secrets never reach the document");
        assert_eq!(doc["paths"]["/_brain/files/invoices"]["post"]["x-brain-bucket"]["types"], json!(["application/pdf"]));
        assert!(doc["paths"]["/_brain/files/invoices/{id}"]["get"].is_object());
        assert!(doc["paths"]["/api/products"]["get"].is_object(), "resource paths are kept");
        assert!(doc["components"]["schemas"]["BrainFile"].is_object());
    }
}
