use crate::exports::heyreach::client::api::{ApiError, ApiErrorCode};
use crate::wasi::http::outgoing_handler;
use crate::wasi::http::types::*;
use serde::de::DeserializeOwned;
use serde::Serialize;

const BASE_URL: &str = "https://api.heyreach.io";

pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

pub fn make_request<T: DeserializeOwned>(
    method: HttpMethod,
    path: &str,
    api_key: &str,
    body: Option<&impl Serialize>,
) -> Result<T, ApiError> {
    let url = format!("{}{}", BASE_URL, path);

    let outgoing_request = OutgoingRequest::new(Fields::new());
    let headers = outgoing_request.headers();

    // Set headers
    headers
        .set(&"content-type".to_string(), &[b"application/json".to_vec()])
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to set content-type header"))?;

    headers
        .set(&"x-api-key".to_string(), &[api_key.as_bytes().to_vec()])
        .map_err(|_| api_error(ApiErrorCode::Unauthorized, "Failed to set API key header"))?;

    // Set method
    let method_value = match method {
        HttpMethod::Get => Method::Get,
        HttpMethod::Post => Method::Post,
        HttpMethod::Delete => Method::Delete,
    };
    outgoing_request
        .set_method(&method_value)
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to set method"))?;

    // Set URL
    outgoing_request
        .set_path_with_query(Some(&url))
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to set URL"))?;

    outgoing_request
        .set_scheme(Some(&Scheme::Https))
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to set scheme"))?;

    outgoing_request
        .set_authority(Some("api.heyreach.io"))
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to set authority"))?;

    // Set body if provided
    if let Some(body_data) = body {
        let body_bytes = serde_json::to_vec(body_data).map_err(|e| {
            api_error(
                ApiErrorCode::BadRequest,
                &format!("Failed to serialize body: {}", e),
            )
        })?;

        let outgoing_body = outgoing_request
            .body()
            .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to get outgoing body"))?;

        let body_stream = outgoing_body
            .write()
            .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to get body stream"))?;

        body_stream
            .blocking_write_and_flush(&body_bytes)
            .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to write body"))?;

        drop(body_stream);
        OutgoingBody::finish(outgoing_body, None)
            .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to finish body"))?;
    }

    // Send request
    let future_response = outgoing_handler::handle(outgoing_request, None)
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to send request"))?;

    let incoming_response = future_response
        .get()
        .ok_or_else(|| api_error(ApiErrorCode::Unknown, "Request not completed"))?
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Request failed"))?
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Request error"))?;

    // Get status
    let status = incoming_response.status();

    // Read response body
    let incoming_body = incoming_response
        .consume()
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to get response body"))?;

    let body_stream = incoming_body
        .stream()
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to get body stream"))?;

    let mut response_bytes = Vec::new();
    loop {
        let chunk = body_stream
            .blocking_read(8192)
            .map_err(|_| api_error(ApiErrorCode::Unknown, "Failed to read response"))?;

        if chunk.is_empty() {
            break;
        }
        response_bytes.extend_from_slice(&chunk);
    }

    drop(body_stream);
    IncomingBody::finish(incoming_body);

    // Handle error status codes
    if status >= 400 {
        let error_code = match status {
            401 => ApiErrorCode::Unauthorized,
            404 => ApiErrorCode::NotFound,
            429 => ApiErrorCode::TooManyRequests,
            400 => ApiErrorCode::BadRequest,
            422 => ApiErrorCode::Validation,
            _ => ApiErrorCode::Unknown,
        };

        let error_message = if let Ok(text) = String::from_utf8(response_bytes.clone()) {
            // Try to parse error message from JSON
            if let Ok(error_json) = serde_json::from_str::<serde_json::Value>(&text) {
                error_json
                    .get("detail")
                    .or_else(|| error_json.get("errorMessage"))
                    .or_else(|| error_json.get("message"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("HTTP {}", status))
            } else {
                text
            }
        } else {
            format!("HTTP {}", status)
        };

        return Err(api_error(error_code, &error_message));
    }

    // Parse response
    let response_text = String::from_utf8(response_bytes)
        .map_err(|_| api_error(ApiErrorCode::Unknown, "Invalid UTF-8 in response"))?;

    serde_json::from_str(&response_text).map_err(|e| {
        api_error(
            ApiErrorCode::Unknown,
            &format!("Failed to parse response: {}", e),
        )
    })
}

pub fn make_request_empty(
    method: HttpMethod,
    path: &str,
    api_key: &str,
    body: Option<&impl Serialize>,
) -> Result<(), ApiError> {
    let _: serde_json::Value = make_request(method, path, api_key, body)?;
    Ok(())
}

fn api_error(code: ApiErrorCode, message: &str) -> ApiError {
    ApiError {
        code,
        message: message.to_string(),
    }
}
