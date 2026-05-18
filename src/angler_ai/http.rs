use crate::prelude::*;
use fish_widestring::{WString, bytes2wcstring};
use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

pub(crate) enum AuthStyle {
    Bearer,
}

pub(crate) fn post_json(
    url: &str,
    auth: Option<(&str, AuthStyle)>,
    body: &Value,
) -> Result<Value, WString> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| {
            sprintf!(
                "Failed to initialize Angler AI HTTP client: %s",
                err.to_string()
            )
        })?;
    let mut request = client.post(url.to_owned());
    if let Some((credential, AuthStyle::Bearer)) = auth {
        request = request.bearer_auth(credential);
    }
    let response = request
        .json(body)
        .send()
        .map_err(|err| sprintf!("Angler AI request failed: %s", err.to_string()))?;
    let status = response.status();
    let response_text = response
        .text()
        .map_err(|err| sprintf!("Failed to read Angler AI response: %s", err.to_string()))?;
    let response_json: Value = serde_json::from_str(&response_text).map_err(|err| {
        sprintf!(
            "Failed to parse Angler AI response JSON: %s",
            err.to_string()
        )
    })?;

    if !status.is_success() {
        if let Some(message) = extract_error_message(&response_json) {
            return Err(bytes2wcstring(message.as_bytes()));
        }
        return Err(sprintf!(
            "Angler AI request failed with HTTP status %s",
            status.to_string()
        ));
    }
    Ok(response_json)
}

pub(crate) fn trim_response_content(content: &str) -> WString {
    let mut result = bytes2wcstring(content.as_bytes());
    while result.ends_with('\n') || result.ends_with('\r') {
        result.pop();
    }
    result
}

fn extract_error_message(response_json: &Value) -> Option<&str> {
    response_json
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| response_json.get("message").and_then(Value::as_str))
}
