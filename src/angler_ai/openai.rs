use super::{
    Config,
    http::{AuthStyle, post_json, trim_response_content},
    system_prompt,
};
use crate::prelude::*;
use fish_widestring::{WString, wcs2bytes};
use serde_json::{Value, json};

pub(crate) fn request(config: Config, prompt: &wstr) -> Result<WString, WString> {
    let api_key = String::from_utf8_lossy(&wcs2bytes(&config.api_key)).into_owned();
    let model = String::from_utf8_lossy(&wcs2bytes(&config.model)).into_owned();
    let user_prompt = String::from_utf8_lossy(&wcs2bytes(prompt)).into_owned();
    let url = config.chat_completions_url()?;
    let body = json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt()
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "temperature": 0.2,
        "stream": false
    });
    let response_json = post_json(&url, Some((&api_key, AuthStyle::Bearer)), &body)?;
    parse_response(&response_json)
}

fn parse_response(response_json: &Value) -> Result<WString, WString> {
    let Some(content) = response_json
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
    else {
        return Err(
            L!("Angler AI response did not include choices[0].message.content.").to_owned(),
        );
    };
    Ok(trim_response_content(content))
}
