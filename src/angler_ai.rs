use crate::prelude::*;
use fish_widestring::{WString, bytes2wcstring, wcs2bytes};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Loading,
    Ready(WString),
    Error(WString),
}

#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: WString,
    pub api_key: WString,
    pub model: WString,
}

impl Config {
    pub fn new(
        base_url: Option<WString>,
        api_key: Option<WString>,
        model: Option<WString>,
    ) -> Result<Self, WString> {
        let Some(api_key) = api_key else {
            return Err(L!("Set ANGLER_AI_KEY to your OpenAI-compatible API key.").to_owned());
        };
        Ok(Self {
            base_url: base_url.unwrap_or_else(|| L!("https://api.openai.com").to_owned()),
            api_key,
            model: model.unwrap_or_else(|| L!("gpt-5.2").to_owned()),
        })
    }

    fn chat_completions_url(&self) -> Result<String, WString> {
        let mut base_url = String::from_utf8_lossy(&wcs2bytes(&self.base_url)).into_owned();
        base_url.truncate(base_url.trim_end_matches('/').len());
        if base_url.is_empty() {
            return Err(L!("ANGLER_AI_BASE_URL is empty.").to_owned());
        }
        if base_url.ends_with("/v1") {
            Ok(format!("{base_url}/chat/completions"))
        } else {
            Ok(format!("{base_url}/v1/chat/completions"))
        }
    }
}

pub struct Session {
    state: State,
}

impl Session {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn set_loading(&mut self) {
        self.state = State::Loading;
    }

    pub fn reset(&mut self) {
        self.state = State::Idle;
    }
}

pub fn request(config: Config, prompt: &wstr) -> State {
    match request_chat_completion(config, prompt) {
        Ok(result) => State::Ready(result),
        Err(err) => State::Error(err),
    }
}

pub fn prompt_prefix(state: &State) -> &'static wstr {
    match state {
        State::Idle => L!("· AI "),
        State::Loading => L!("⠸ AI "),
        State::Ready(_) => L!("✓ AI "),
        State::Error(_) => L!("! AI "),
    }
}

fn system_prompt() -> &'static str {
    concat!(
        "You are Angler AI, an assistant embedded in an interactive fish shell.\n",
        "The user wrote a natural-language request or a partial shell command.\n",
        "Convert it into one safe fish shell command to insert into the command line.\n\n",
        "Rules:\n",
        "- Output only the command to insert.\n",
        "- Do not explain.\n",
        "- Do not wrap the command in Markdown.\n",
        "- Do not execute anything.\n",
        "- Use fish-compatible syntax, not bash-only syntax.\n",
        "- Prefer non-destructive commands when possible.\n",
        "- If the request is ambiguous or unsafe, output a commented fish command starting with '# ' that asks for clarification.\n"
    )
}

fn request_chat_completion(config: Config, prompt: &wstr) -> Result<WString, WString> {
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

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| {
            sprintf!(
                "Failed to initialize Angler AI HTTP client: %s",
                err.to_string()
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
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
        if let Some(message) = response_json
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
        {
            return Err(bytes2wcstring(message.as_bytes()));
        }
        return Err(sprintf!(
            "Angler AI request failed with HTTP status %s",
            status.to_string()
        ));
    }

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

    let mut result = bytes2wcstring(content.as_bytes());
    while result.ends_with('\n') || result.ends_with('\r') {
        result.pop();
    }
    Ok(result)
}
