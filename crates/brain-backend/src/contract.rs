//! One resource contract drives validation, policy, storage and OpenAPI. No application code is evaluated here.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub kind: String,
    #[serde(default)]
    pub required: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub read: Vec<String>,
    pub create: Vec<String>,
    pub update: Vec<String>,
    pub delete: Vec<String>,
}
impl Default for Policy {
    fn default() -> Self {
        let admin = vec!["admin".into()];
        Self {
            read: admin.clone(),
            create: admin.clone(),
            update: admin.clone(),
            delete: admin,
        }
    }
}
impl Policy {
    pub fn allows(&self, operation: &str, role: Option<&str>, owns: bool) -> bool {
        let grants = match operation {
            "read" => &self.read,
            "create" => &self.create,
            "update" => &self.update,
            "delete" => &self.delete,
            _ => return false,
        };
        grants.iter().any(|r| {
            r == "public" && operation == "read"
                || r == "owner" && owns && role.is_some()
                || role == Some(r.as_str())
        })
    }
    pub fn owners_only(&self, operation: &str, role: Option<&str>) -> bool {
        !self.allows(operation, role, false) && self.allows(operation, role, true)
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    pub before_create: Option<String>,
    pub before_update: Option<String>,
    pub before_delete: Option<String>,
    pub after_create: Option<String>,
    pub after_update: Option<String>,
    pub after_delete: Option<String>,
}
impl Hooks {
    pub fn before(&self, operation: &str) -> Option<&str> {
        match operation {
            "create" => self.before_create.as_deref(),
            "update" => self.before_update.as_deref(),
            "delete" => self.before_delete.as_deref(),
            _ => None,
        }
    }
    pub fn queue(&self, operation: &str) -> Option<&str> {
        match operation {
            "create" => self.after_create.as_deref(),
            "update" => self.after_update.as_deref(),
            "delete" => self.after_delete.as_deref(),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub name: String,
    pub fields: BTreeMap<String, Field>,
    /// Explicit searchable text fields. Empty means no resource values enter the search index.
    #[serde(default)]
    pub search: Vec<String>,
    pub policy: Policy,
    #[serde(default)]
    pub hooks: Hooks,
}

pub fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && !matches!(
            s,
            "constructor"
                | "prototype"
                | "__proto__"
                | "id"
                | "owner"
                | "created_at"
                | "updated_at"
                | "search"
        )
}
impl Resource {
    pub fn validate(&self) -> Result<(), String> {
        if !identifier(&self.name) {
            return Err("resource name must be lowercase letters, digits and underscores, starting with a letter (max 48)".into());
        }
        if self.fields.is_empty() || self.fields.len() > 64 {
            return Err("a resource needs 1..64 fields".into());
        }
        for (name, field) in &self.fields {
            if !identifier(name)
                || !matches!(
                    field.kind.as_str(),
                    "str" | "num" | "bool" | "email" | "obj"
                )
            {
                return Err(format!(
                    "invalid field `{name}`; fix: use name:str|num|bool|email|obj with optional !"
                ));
            }
        }
        let search_unique: std::collections::HashSet<_> = self.search.iter().collect();
        if self.search.len() > 8
            || search_unique.len() != self.search.len()
            || self
                .search
                .iter()
                .any(|name| !self.fields.get(name).is_some_and(|f| f.kind == "str"))
        {
            return Err("search must contain at most 8 unique declared str fields; private and object fields are excluded unless explicitly allowed text fields".into());
        }
        for queue in [
            &self.hooks.before_create,
            &self.hooks.before_update,
            &self.hooks.before_delete,
            &self.hooks.after_create,
            &self.hooks.after_update,
            &self.hooks.after_delete,
        ]
        .into_iter()
        .flatten()
        {
            if !identifier(queue) {
                return Err("hook names and queues must be valid identifiers".into());
            }
        }
        for (op, grants) in [
            ("read", &self.policy.read),
            ("create", &self.policy.create),
            ("update", &self.policy.update),
            ("delete", &self.policy.delete),
        ] {
            for role in grants {
                if !(matches!(role.as_str(), "owner" | "public")
                    || crate::auth_flows::role_name(role))
                    || role == "public" && op != "read"
                    || role == "owner" && op == "create"
                {
                    return Err(format!(
                        "invalid {op} policy `{role}`; writes need an explicit role; owner applies to existing rows"
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn input(&self, value: &Value) -> Result<Value, BTreeMap<String, String>> {
        let mut errors = BTreeMap::new();
        let Some(obj) = value.as_object() else {
            return Err(BTreeMap::from([(
                "body".into(),
                "Expected an object".into(),
            )]));
        };
        let mut clean = Map::new();
        for key in obj.keys() {
            if !self.fields.contains_key(key) {
                errors.insert(key.clone(), "Unknown field".into());
            }
        }
        for (name, field) in &self.fields {
            let v = obj.get(name).unwrap_or(&Value::Null);
            if v.is_null() {
                if field.required {
                    errors.insert(name.clone(), "Required".into());
                }
                continue;
            }
            let valid = match field.kind.as_str() {
                "str" => v
                    .as_str()
                    .is_some_and(|s| s.len() <= 16384 && (!field.required || !s.trim().is_empty())),
                "email" => v.as_str().is_some_and(|s| {
                    s.len() <= 254
                        && !s.chars().any(char::is_whitespace)
                        && s.split_once('@').is_some_and(|(a, b)| {
                            !a.is_empty() && b.contains('.') && !b.contains('@')
                        })
                }),
                "num" => v.as_f64().is_some_and(f64::is_finite),
                "bool" => v.is_boolean(),
                "obj" => v.is_object() && v.to_string().len() <= 32768,
                _ => false,
            };
            if valid {
                clean.insert(name.clone(), v.clone());
            } else {
                errors.insert(name.clone(), format!("Expected {}", field.kind));
            }
        }
        if errors.is_empty() {
            Ok(Value::Object(clean))
        } else {
            Err(errors)
        }
    }
    pub fn schema(&self) -> Value {
        let props: Map<String, Value> = self.fields.iter().map(|(n,f)| (n.clone(), match f.kind.as_str() {
            "obj" => json!({"type":"object"}), "num" => json!({"type":"number"}), "bool" => json!({"type":"boolean"}),
            "email" => json!({"type":"string","format":"email","maxLength":254}),
            _ => json!({"type":"string","maxLength":16384, "minLength": if f.required {1} else {0}}),
        })).collect();
        json!({"type":"object","additionalProperties":false,"properties":props,"required":self.fields.iter().filter(|(_,f)| f.required).map(|(n,_)|n).collect::<Vec<_>>()})
    }
}

pub fn openapi(resources: &[Resource]) -> Value {
    openapi_with_roles(resources, &crate::auth_flows::RolesConfig::default())
}
pub fn openapi_with_roles(resources: &[Resource], roles: &crate::auth_flows::RolesConfig) -> Value {
    let mut paths = Map::new();
    for r in resources {
        let body = json!({"required":true,"content":{"application/json":{"schema":r.schema()}}});
        let error_responses = json!({"400":{"description":"Invalid request"},"401":{"description":"Sign in required"},"403":{"description":"Policy denied"},"404":{"description":"Not found"},"422":{"description":"Validation errors"},"429":{"description":"Rate limited"}});
        let row = json!({"type":"object","additionalProperties":false,"required":["id","owner","data"],"properties":{"id":{"type":"string","format":"uuid"},"owner":{"type":"string","format":"uuid"},"data":r.schema()}});
        let op = |name: &str, write: bool| {
            let mut out = json!({"operationId":format!("{}_{}",r.name,name),"security":native_security(),"responses":error_responses});
            let output = if name == "list" {
                json!({"type":"object","required":["items","limit","offset"],"properties":{"items":{"type":"array","items":row},"limit":{"type":"integer"},"offset":{"type":"integer"}}})
            } else if name == "delete" {
                json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}})
            } else {
                row.clone()
            };
            out["responses"]["200"] =
                json!({"description":"Success","content":{"application/json":{"schema":output}}});
            let operation = if name == "list" || name == "read" {
                "read"
            } else {
                name
            };
            out["x-brain-policy"] = serde_json::to_value(&r.policy).unwrap()[operation].clone();
            if let Some(hook) = r.hooks.before(operation) {
                out["x-brain-before-hook"] =
                    json!({"name":hook,"required":true,"outputRevalidated":true});
                out["responses"]["503"] = json!({"description":"Mandatory hook unavailable, timed out or at capacity; no mutation committed"});
            }
            if matches!(operation, "update" | "delete") {
                out["x-brain-conflict-check"] = json!("server-row-snapshot");
                out["responses"]["409"] = json!({"description":"Record changed during server validation; reload and retry"});
            }

            if operation == "read" && r.policy.read.iter().any(|role| role == "public") {
                out["security"] = json!([]);
            }
            if name == "list" {
                out["parameters"] = json!([
                    {"name":"search","in":"query","schema":{"type":"string","maxLength":256}},
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1,"maximum":100,"default":25}},
                    {"name":"offset","in":"query","schema":{"type":"integer","minimum":0,"maximum":10000,"default":0}}
                ]);
            }

            if write {
                out["requestBody"] = body.clone();
            }
            out
        };
        paths.insert(
            format!("/api/{}", r.name),
            json!({"get":op("list",false),"post":op("create",true)}),
        );
        paths.insert(format!("/api/{}/{{id}}",r.name),json!({"parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string","format":"uuid"}}],"get":op("read",false),"put":op("update",true),"delete":op("delete",false)}));
    }
    paths.extend(native_paths(roles));
    json!({"openapi":"3.1.0","info":{"title":"BRAINLLM resource API (experimental)","version":env!("CARGO_PKG_VERSION")},"paths":paths,"components":{"securitySchemes":{"session":{"type":"apiKey","in":"cookie","name":"__Host-brain"},"accessCookie":{"type":"apiKey","in":"cookie","name":"__Host-brain-access"},"refreshCookie":{"type":"apiKey","in":"cookie","name":"__Host-brain-refresh"},"googleNonce":{"type":"apiKey","in":"cookie","name":"__Host-brain-google"},"bearer":{"type":"http","scheme":"bearer","bearerFormat":"JWT"}}}})
}

fn native_security() -> Value {
    json!([{"session":[]},{"accessCookie":[]},{"bearer":[]}])
}
fn object(properties: Value, required: &[&str], strict: bool) -> Value {
    let mut schema = json!({"type":"object","properties":properties,"required":required});
    if strict {
        schema["additionalProperties"] = json!(false)
    }
    schema
}
fn body(schema: Value) -> Value {
    json!({"required":true,"content":{"application/json":{"schema":schema}}})
}
fn native_paths(roles: &crate::auth_flows::RolesConfig) -> Map<String, Value> {
    let mut paths = Map::new();
    let id = json!({"type":"string","format":"uuid"});
    let user = object(
        json!({"id":id,"email":{"type":"string","maxLength":254},"role":{"type":"string","enum":roles.roles},"verified":{"type":"boolean"}}),
        &["id", "email", "role", "verified"],
        true,
    );
    let account = object(json!({"user":user}), &["user"], true);
    let tokens = object(
        json!({"user":user,"access_token":{"type":"string","writeOnly":false},"token_type":{"const":"Bearer"},"expires_in":{"const":300}}),
        &["user", "access_token", "token_type", "expires_in"],
        true,
    );
    let credentials = object(
        json!({"email":{"type":"string","format":"email","maxLength":254,"x-brain-normalization":"trim and ASCII lowercase"},"password":{"type":"string","writeOnly":true,"maxLength":1024,"x-brain-maxBytes":1024}}),
        &["email", "password"],
        true,
    );
    let mut signup = credentials.clone();
    signup["properties"]["password"]["x-brain-minBytes"] = json!(12);
    let error_schema = json!({"type":"object","description":"Application error or middleware rejection. X-Request-Id is always authoritative; custom JSON fields may vary.","properties":{"schemaVersion":{"type":"integer"},"rid":{"type":["string","null"]},"error":{"oneOf":[{"type":"string"},{"type":"object","properties":{"code":{"type":"string"},"fix":{"type":"string"},"fields":{}}}]}}});
    let mut add = |path: &str,
                   method: &str,
                   summary: &str,
                   security: Value,
                   success: u16,
                   schema: Option<Value>,
                   request: Option<Value>,
                   errors: &[u16]| {
        let mut responses = Map::new();
        for code in errors {
            let code = code.to_string();
            responses.insert(code.clone(),json!({"description":format!("HTTP {code}; safe application or middleware rejection"),"headers":{"X-Request-Id":{"required":true,"schema":{"type":"string"}}},"content":{"application/json":{"schema":error_schema}}}));
        }
        if !path.starts_with("/_brain/") && !path.starts_with("/api/") {
            for response in responses.values_mut() {
                response["content"]["text/html"] = json!({"schema":{"type":"string"},"description":"Negotiated generic error page; no private exception or request path is exposed"});
            }
        }
        let mut ok = json!({"description":"Success","headers":{"X-Request-Id":{"required":true,"schema":{"type":"string"}}}});
        if let Some(schema) = schema {
            ok["content"] = json!({"application/json":{"schema":schema}})
        }
        responses.insert(success.to_string(), ok);
        let mut operation = json!({"summary":summary,"operationId":format!("native_{}_{}",method,path.trim_matches('/').replace('/',"_").replace(['{','}'],"")),"security":security,"x-brain-native":true,"responses":responses});
        if let Some(request) = request {
            operation["requestBody"] = request
        }
        if matches!(method, "post" | "put" | "patch" | "delete") {
            operation["parameters"] = json!([{"name":"Origin","in":"header","required":true,"description":"Exact configured site origin; browsers supply this automatically","schema":{"type":"string","format":"uri"}}])
        }
        paths.entry(path.to_string()).or_insert_with(|| json!({}))[method] = operation;
    };
    let auth = native_security();
    let open = json!([]);
    add(
        "/health",
        "get",
        "Database health",
        open.clone(),
        200,
        Some(object(
            json!({"ok":{"const":true},"schemaVersion":{"const":1}}),
            &["ok", "schemaVersion"],
            true,
        )),
        None,
        &[429, 503, 504],
    );
    add(
        "/_brain/api",
        "get",
        "Runtime API contract; production requires an administrator",
        auth.clone(),
        200,
        Some(json!({"type":"object"})),
        None,
        &[401, 403, 429, 500, 504],
    );
    add(
        "/_brain/auth/signup",
        "post",
        "Create an unverified candidate account",
        open.clone(),
        200,
        Some(account.clone()),
        Some(body(signup)),
        &[400, 403, 413, 415, 422, 429, 504],
    );
    add(
        "/_brain/auth/login",
        "post",
        "Sign in using backend-selected JWT cookies when configured, otherwise a revocable opaque session; no fallback on authentication failure",
        open.clone(),
        200,
        Some(json!({"oneOf":[account.clone(),tokens.clone()]})),
        Some(body(credentials.clone())),
        &[400, 401, 403, 413, 415, 422, 429, 503, 504],
    );
    add(
        "/_brain/auth/logout",
        "post",
        "Revoke supplied native sessions and clear auth cookies; authentication is optional",
        open.clone(),
        200,
        Some(object(json!({"ok":{"const":true}}), &["ok"], true)),
        None,
        &[403, 429, 500, 504],
    );
    add(
        "/_brain/auth/me",
        "get",
        "Read authoritative current user",
        auth.clone(),
        200,
        Some(account),
        None,
        &[401, 429, 500, 504],
    );
    add(
        "/_brain/auth/token",
        "post",
        "Configured JWT access (five minutes) and HttpOnly rotating refresh cookie",
        open.clone(),
        200,
        Some(tokens.clone()),
        Some(body(credentials)),
        &[400, 401, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/refresh",
        "post",
        "Rotate the single-use HttpOnly refresh cookie",
        json!([{"refreshCookie":[]}]),
        200,
        Some(tokens.clone()),
        None,
        &[401, 403, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/google/challenge",
        "post",
        "Configured public Google client id and one-use browser-bound five-minute nonce",
        open.clone(),
        200,
        Some(object(
            json!({"client_id":{"type":"string"},"nonce":{"type":"string","pattern":"^[a-f0-9]{64}$"},"expires_in":{"const":300}}),
            &["client_id", "nonce", "expires_in"],
            true,
        )),
        None,
        &[403, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/google",
        "post",
        "Verify configured GIS ID token and nonce; never link by email",
        json!([{"googleNonce":[]}]),
        200,
        Some(tokens),
        Some(body(object(
            json!({"credential":{"type":"string","maxLength":16384,"writeOnly":true}}),
            &["credential"],
            true,
        ))),
        &[400, 401, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/slugs",
        "post",
        "Normalize a bounded public title into a slug",
        auth.clone(),
        200,
        Some(object(
            json!({"schemaVersion":{"const":1},"slug":{"type":"string","maxLength":128}}),
            &["schemaVersion", "slug"],
            true,
        )),
        Some(body(object(
            json!({"title":{"type":"string","maxLength":4096,"x-brain-maxBytes":4096},"limit":{"type":["integer","null"],"minimum":8,"maximum":128,"default":80}}),
            &["title"],
            true,
        ))),
        &[400, 401, 403, 413, 415, 422, 429, 500, 504],
    );
    add(
        "/_brain/links",
        "post",
        "Create an owned short link restricted to configured application origins",
        auth.clone(),
        201,
        Some(object(
            json!({"id":id,"code":{"type":"string","pattern":"^[a-z0-9][a-z0-9-]{0,79}$"},"path":{"type":"string"}}),
            &["id", "code", "path"],
            true,
        )),
        Some(body(object(
            json!({"target":{"type":"string","format":"uri","maxLength":2048,"x-brain-maxBytes":2048},"code":{"type":["string","null"],"pattern":"^[a-z0-9][a-z0-9-]{0,79}$"},"expires_in":{"type":["integer","null"],"minimum":60,"maximum":31536000,"default":2592000}}),
            &["target"],
            true,
        ))),
        &[400, 401, 403, 413, 415, 422, 429, 500, 504],
    );
    add(
        "/_brain/links/{id}",
        "delete",
        "Revoke an owned short link",
        auth.clone(),
        204,
        None,
        None,
        &[400, 401, 403, 404, 429, 500, 504],
    );
    add(
        "/s/{code}",
        "get",
        "Resolve a live link using a non-cacheable redirect",
        open.clone(),
        302,
        None,
        None,
        &[404, 429, 500, 504],
    );
    let uploaded = object(
        json!({"id":id,"mediaType":{"type":"string","enum":["image/png","image/jpeg","image/webp"]},"bytes":{"type":"integer","minimum":1,"maximum":20971520},"width":{"type":"integer","minimum":1,"maximum":4096},"height":{"type":"integer","minimum":1,"maximum":4096},"version":{"const":1},"url":{"type":"string"}}),
        &[
            "id",
            "mediaType",
            "bytes",
            "width",
            "height",
            "version",
            "url",
        ],
        true,
    );
    add(
        "/_brain/uploads",
        "post",
        "Validate and privately store the original plus bounded versioned image renditions",
        auth.clone(),
        200,
        Some(uploaded),
        Some(
            json!({"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["file"],"additionalProperties":false,"properties":{"file":{"type":"string","format":"binary","x-brain-maxBytes":5242880,"description":"Content-validated PNG/JPEG/WebP; no client filename paths"}}}}}}),
        ),
        &[400, 401, 403, 409, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/uploads/{id}",
        "get",
        "Authorized private attachment; compatible with legacy flat images",
        auth.clone(),
        200,
        None,
        None,
        &[400, 401, 403, 404, 429, 500, 503, 504],
    );
    add(
        "/_brain/media",
        "get",
        "Current user's ready image library and responsive srcsets",
        auth.clone(),
        200,
        Some(object(
            json!({"items":{"type":"array","maxItems":100,"items":{"type":"object","description":"id, mediaType, bytes, width, height, alt, caption, version, url, renditions and format-specific srcset; private object keys are excluded"}},"limit":{"type":"integer"},"offset":{"type":"integer"},"nextOffset":{"type":["integer","null"]}}),
            &["items", "limit", "offset", "nextOffset"],
            true,
        )),
        None,
        &[400, 401, 422, 429, 500, 504],
    );
    add(
        "/_brain/media/{id}",
        "patch",
        "Update owner/admin plain-text alt and caption",
        auth.clone(),
        200,
        Some(object(
            json!({"id":id,"ok":{"const":true}}),
            &["id", "ok"],
            true,
        )),
        Some(body(object(
            json!({"alt":{"type":"string","maxLength":1024,"x-brain-maxBytes":1024},"caption":{"type":"string","maxLength":4096,"x-brain-maxBytes":4096}}),
            &["alt", "caption"],
            true,
        ))),
        &[400, 401, 403, 404, 413, 415, 422, 429, 500, 504],
    );
    add(
        "/_brain/media/{id}",
        "delete",
        "Remove owner/admin metadata access and queue durable object cleanup",
        auth.clone(),
        202,
        Some(object(
            json!({"id":id,"cleanupQueued":{"const":true}}),
            &["id", "cleanupQueued"],
            true,
        )),
        None,
        &[400, 401, 403, 404, 429, 500, 504],
    );
    let preset = object(
        json!({"name":{"type":"string","maxLength":48,"pattern":"^[a-z0-9_-]+$","not":{"enum":["original","full","thumbnail"]}},"width":{"type":"integer","minimum":1,"maximum":4096},"height":{"type":"integer","minimum":1,"maximum":4096},"crop":{"type":"boolean","default":false},"format":{"type":"string","enum":["png","jpeg","webp"]},"quality":{"type":["integer","null"],"minimum":1,"maximum":100,"description":"Only valid for JPEG; WebP is lossless"}}),
        &["name", "width", "height", "format"],
        true,
    );
    add(
        "/_brain/media/{id}/variants",
        "post",
        "Rebuild a new immutable version from the preserved original",
        auth.clone(),
        200,
        Some(object(
            json!({"id":id,"version":{"type":"integer","minimum":2,"maximum":64},"previousVersion":{"type":"integer"},"url":{"type":"string"}}),
            &["id", "version", "previousVersion", "url"],
            true,
        )),
        Some(body(object(
            json!({"presets":{"type":"array","maxItems":6,"items":preset,"description":"Omit for defaults; names must be unique"}}),
            &[],
            true,
        ))),
        &[400, 401, 403, 404, 409, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/media/{id}/v/{version}/{preset}",
        "get",
        "Authorized immutable image version; original always downloads as an attachment",
        auth.clone(),
        200,
        None,
        None,
        &[400, 401, 404, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/jobs/{queue}",
        "post",
        "Administrator-only validated durable enqueue",
        auth.clone(),
        200,
        Some(object(
            json!({"schemaVersion":{"const":1},"id":id}),
            &["schemaVersion", "id"],
            true,
        )),
        Some(body(object(
            json!({"payload":{"type":"object","description":"Must match the configured queue's server/jobs schema"},"idempotency_key":{"type":["string","null"],"minLength":1,"maxLength":128,"x-brain-maxBytes":128},"max_attempts":{"type":["integer","null"],"minimum":1,"maximum":10,"default":3}}),
            &["payload"],
            true,
        ))),
        &[400, 401, 403, 404, 413, 415, 422, 429, 500, 504],
    );
    add(
        "/_brain/jobs/status/{id}",
        "get",
        "Administrator-only job state without payload or private exception traces",
        auth,
        200,
        Some(object(
            json!({"id":id,"queue":{"type":"string"},"state":{"type":"string","enum":["pending","running","succeeded","dead"]},"attempts":{"type":"integer","minimum":0,"maximum":10},"maxAttempts":{"type":"integer","minimum":1,"maximum":10},"lastError":{"type":["string","null"]}}),
            &[
                "id",
                "queue",
                "state",
                "attempts",
                "maxAttempts",
                "lastError",
            ],
            true,
        )),
        None,
        &[400, 401, 403, 404, 429, 500, 504],
    );
    add(
        "/_brain/auth/password/request",
        "post",
        "Request a single-use account recovery link; uniform accepted response does not identify accounts",
        open.clone(),
        202,
        Some(object(
            json!({"accepted":{"const":true}}),
            &["accepted"],
            true,
        )),
        Some(body(object(
            json!({"email":{"type":"string","format":"email","maxLength":254}}),
            &["email"],
            true,
        ))),
        &[400, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/password/reset",
        "post",
        "Consume a recovery token once, update password and revoke all user sessions",
        open.clone(),
        200,
        Some(object(json!({"ok":{"const":true}}), &["ok"], true)),
        Some(body(object(
            json!({"token":{"type":"string","pattern":"^[a-f0-9]{64}$","writeOnly":true},"password":{"type":"string","writeOnly":true,"x-brain-minBytes":12,"x-brain-maxBytes":1024}}),
            &["token", "password"],
            true,
        ))),
        &[400, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/email/request",
        "post",
        "Send verification to the current authenticated account email",
        native_security(),
        202,
        Some(object(
            json!({"accepted":{"const":true}}),
            &["accepted"],
            true,
        )),
        Some(body(object(json!({}), &[], true))),
        &[400, 401, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/email/verify",
        "post",
        "Consume one current-user and email-bound verification token",
        native_security(),
        200,
        Some(object(json!({"ok":{"const":true}}), &["ok"], true)),
        Some(body(object(
            json!({"token":{"type":"string","pattern":"^[a-f0-9]{64}$","writeOnly":true}}),
            &["token"],
            true,
        ))),
        &[400, 401, 403, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/auth/users/{id}/role",
        "post",
        "Current database administrator assigns a configured role and revokes target sessions",
        native_security(),
        200,
        Some(object(json!({"ok":{"const":true}}), &["ok"], true)),
        Some(body(object(
            json!({"role":{"type":"string","enum":roles.roles}}),
            &["role"],
            true,
        ))),
        &[400, 401, 403, 404, 413, 415, 422, 429, 500, 503, 504],
    );
    let receipt = object(
        json!({"id":id,"eventId":id,"duplicate":{"type":"boolean"}}),
        &["id", "eventId", "duplicate"],
        true,
    );
    add(
        "/_brain/webhooks/in/{name}",
        "post",
        "Accept signed exact-byte JSON into a declared queue; signature and timestamp headers are mandatory, cookies grant no authority",
        open.clone(),
        202,
        Some(receipt.clone()),
        Some(body(
            json!({"type":"object","additionalProperties":true,"x-brain-maxBytes":16384,"x-brain-maxDepth":32}),
        )),
        &[400, 403, 404, 409, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/webhooks/out/{name}",
        "post",
        "Administrator explicitly enqueues an encrypted durable outbound delivery; no automatic resource subscription",
        native_security(),
        202,
        Some(receipt),
        Some(body(object(
            json!({"eventId":id,"payload":{"type":"object","additionalProperties":true,"x-brain-maxBytes":16384,"x-brain-maxDepth":32}}),
            &["eventId", "payload"],
            true,
        ))),
        &[400, 401, 403, 404, 409, 413, 415, 422, 429, 500, 503, 504],
    );
    add(
        "/_brain/webhooks/out/{name}/{id}",
        "get",
        "Administrator reads delivery state and bounded static outcomes without URL, headers or body",
        native_security(),
        200,
        Some(object(
            json!({"id":id,"eventId":id,"state":{"type":"string","enum":["pending","running","succeeded","dead"]},"attempts":{"type":"integer","minimum":0,"maximum":5},"maxAttempts":{"const":5},"deliveries":{"type":"array","maxItems":5,"items":{"type":"object","properties":{"attempt":{"type":"integer","minimum":1,"maximum":5},"status":{"type":["integer","null"]},"code":{"type":"string","enum":["accepted","http-rejected","transport-failed","deadline"]},"ms":{"type":"integer","minimum":0,"maximum":5000}},"required":["attempt","status","code","ms"],"additionalProperties":false}}}),
            &[
                "id",
                "eventId",
                "state",
                "attempts",
                "maxAttempts",
                "deliveries",
            ],
            true,
        )),
        None,
        &[400, 401, 403, 404, 429, 500, 503, 504],
    );
    drop(add);
    paths["/_brain/webhooks/in/{name}"]["post"]["x-brain-signature"] = json!({"algorithm":"HMAC-SHA256","frame":"brain-webhook-v1\n{event-id}\n{timestamp}\n{raw-body}","timestampToleranceSeconds":300,"originRequired":false});
    paths["/_brain/webhooks/in/{name}"]["post"]["parameters"] = json!([
      {"name":"X-Brain-Webhook-ID","in":"header","required":true,"schema":{"type":"string","format":"uuid"}},
      {"name":"X-Brain-Webhook-Timestamp","in":"header","required":true,"schema":{"type":"string","pattern":"^(0|[1-9][0-9]*)$"}},
      {"name":"X-Brain-Webhook-Signature","in":"header","required":true,"schema":{"type":"string","pattern":"^v1=[a-fA-F0-9]{64}$"}}
    ]);
    for (path, item) in &mut paths {
        let mut parameters = Vec::new();
        for part in path.split('/') {
            if let Some(name) = part
                .strip_prefix('{')
                .and_then(|part| part.strip_suffix('}'))
            {
                let schema = match name {
                    "id" => id.clone(),
                    "version" => json!({"type":"integer","minimum":1,"maximum":64}),
                    "name" => json!({"type":"string","pattern":"^[a-z][a-z0-9_]{0,47}$"}),
                    "queue" => json!({"type":"string","pattern":"^[a-z][a-z0-9_]{0,47}$"}),
                    "code" => json!({"type":"string","pattern":"^[a-z0-9][a-z0-9-]{0,79}$"}),
                    "preset" => json!({"type":"string","pattern":"^[a-z0-9_-]{1,48}$"}),
                    _ => json!({"type":"string"}),
                };
                parameters.push(json!({"name":name,"in":"path","required":true,"schema":schema}));
            }
        }
        if !parameters.is_empty() {
            item["parameters"] = json!(parameters)
        }
    }
    paths.get_mut("/s/{code}").unwrap()["get"]["responses"]["302"]["headers"]["Location"] =
        json!({"required":true,"schema":{"type":"string","format":"uri"}});
    paths.get_mut("/s/{code}").unwrap()["get"]["responses"]["302"]["headers"]["Cache-Control"] =
        json!({"schema":{"const":"no-store"}});
    paths.get_mut("/_brain/media").unwrap()["get"]["parameters"] = json!([{"name":"limit","in":"query","schema":{"type":"integer","minimum":1,"maximum":100,"default":24}},{"name":"offset","in":"query","schema":{"type":"integer","minimum":0,"maximum":10000,"default":0}}]);
    paths.get_mut("/_brain/uploads/{id}").unwrap()["get"]["parameters"] =
        json!([{"name":"thumbnail","in":"query","schema":{"type":"boolean","default":false}}]);
    for path in [
        "/_brain/uploads/{id}",
        "/_brain/media/{id}/v/{version}/{preset}",
    ] {
        paths.get_mut(path).unwrap()["get"]["responses"]["200"]["content"] = json!({"image/png":{"schema":{"type":"string","format":"binary"}},"image/jpeg":{"schema":{"type":"string","format":"binary"}},"image/webp":{"schema":{"type":"string","format":"binary"}}});
    }
    for path in [
        "/_brain/jobs/{queue}",
        "/_brain/jobs/status/{id}",
        "/_brain/webhooks/out/{name}",
        "/_brain/webhooks/out/{name}/{id}",
    ] {
        for method in ["get", "post"] {
            if paths.get_mut(path).unwrap().get(method).is_some() {
                paths.get_mut(path).unwrap()[method]["x-brain-required-role"] = json!("admin")
            }
        }
    }
    for item in paths.values_mut() {
        if let Some(get) = item.get("get").cloned() {
            let mut head = get;
            head["summary"] = json!(format!(
                "HEAD: {}",
                head["summary"].as_str().unwrap_or("response")
            ));
            head["operationId"] = json!(head["operationId"].as_str().unwrap().replacen(
                "native_get_",
                "native_head_",
                1
            ));
            for response in head["responses"].as_object_mut().unwrap().values_mut() {
                response.as_object_mut().unwrap().remove("content");
            }
            item["head"] = head;
        }
    }
    paths
}

/// A hook is a typed event, not an arbitrary payload sent to an unrelated queue.
pub fn validate_graph(resources: &[Resource], jobs: &[Resource]) -> Result<(), String> {
    for r in resources.iter().chain(jobs) {
        r.validate()?;
    }
    for r in resources {
        for operation in ["create", "update", "delete"] {
            if let Some(name) = r.hooks.queue(operation) {
                let job = jobs
                    .iter()
                    .find(|j| j.name == name)
                    .ok_or_else(|| format!("{} hook references missing job `{name}`", r.name))?;
                if job
                    .input(&json!({"resource":r.name,"operation":operation,"record":{}}))
                    .is_err()
                {
                    return Err(format!(
                        "{name}: a resource hook job must accept resource:str operation:str record:obj, with no other required fields"
                    ));
                }
            }
        }
    }
    Ok(())
}
