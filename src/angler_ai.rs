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
pub enum Provider {
    OpenAiCompatible,
    RwkvLightning,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: WString,
    pub api_key: WString,
    pub model: WString,
    pub provider: Provider,
}

impl Config {
    pub fn new(
        base_url: Option<WString>,
        api_key: Option<WString>,
        model: Option<WString>,
        provider: Option<WString>,
    ) -> Result<Self, WString> {
        let Some(api_key) = api_key else {
            return Err(
                L!("Set ANGLER_AI_KEY to your API key or RWKV Lightning password.").to_owned(),
            );
        };
        let model = model.unwrap_or_else(|| L!("gpt-5.2").to_owned());
        Ok(Self {
            base_url: base_url.unwrap_or_else(|| L!("https://api.openai.com").to_owned()),
            api_key,
            provider: parse_provider(provider, &model)?,
            model,
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
    match request_completion(config, prompt) {
        Ok(result) => State::Ready(result),
        Err(err) => State::Error(err),
    }
}

pub fn prompt_prefix(state: &State) -> &'static wstr {
    match state {
        State::Idle => L!("🟢 🕊️ "),
        State::Loading => L!("✳️ 🕊️ "),
        State::Ready(_) => L!("✅ 🕊️ "),
        State::Error(_) => L!("❗ 🕊️ "),
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

fn rwkv_prompt(prompt: &str) -> String {
    format!(
        concat!(
            "System: You are a fish shell assistant.Format:```fish\\nCOMMANDS\\n```\n",
            "Rules:\n",
            "* Only output one fish code block\n",
            "* No text before the code block\n",
            "* No text after the code block\n",
            "* No explanation\n",
            "* No markdown\n",
            "* Use valid fish shell syntax\n",
            "* Keep commands minimal and efficient.\n\n",
            "User: 查看当前目录大小\n\n",
            "Assistant: ```fish\n",
            "du -sh .\n",
            "```\n\n",
            "User: 查看Nvidia GPU状态\n\n",
            "Assistant: ```fish\n",
            "nvtop\n",
            "```\n\n",
            "User: 查找大于1GB的文件\n\n",
            "Assistant: ```fish\n",
            "find . -type f -size +1G\n",
            "```\n\n",
            "User: 查看端口占用\n\n",
            "Assistant: ```fish\n",
            "ss -tulpn\n",
            "```\n\n",
            "User: {}\n\n",
            "Assistant: ```fish\n"
        ),
        prompt
    )
}

fn request_completion(config: Config, prompt: &wstr) -> Result<WString, WString> {
    match config.provider {
        Provider::OpenAiCompatible => request_openai_chat_completion(config, prompt),
        Provider::RwkvLightning => request_rwkv_completion(config, prompt),
    }
}

fn request_openai_chat_completion(config: Config, prompt: &wstr) -> Result<WString, WString> {
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
    parse_openai_response(&response_json)
}

fn request_rwkv_completion(config: Config, prompt: &wstr) -> Result<WString, WString> {
    let password = String::from_utf8_lossy(&wcs2bytes(&config.api_key)).into_owned();
    let user_prompt = String::from_utf8_lossy(&wcs2bytes(prompt)).into_owned();
    let url = config.chat_completions_url()?;
    let response = request_rwkv_batch(&url, &password, &[rwkv_prompt(&user_prompt)])?;
    response.into_iter().next().ok_or_else(|| {
        L!("RWKV Lightning response did not include choices[0].message.content.").to_owned()
    })
}

fn request_rwkv_batch(
    url: &str,
    password: &str,
    prompts: &[String],
) -> Result<Vec<WString>, WString> {
    let body = json!({
        "contents": prompts,
        "max_tokens": 1024,
        "temperature": 1,
        "top_k": 20,
        "top_p": 0,
        "alpha_presence": 0,
        "alpha_frequency": 0,
        "alpha_decay": 0.99,
        "chunk_size": 8,
        "stream": false,
        "password": password,
    });
    let response_json = post_json(url, None, &body)?;
    parse_rwkv_batch_response(&response_json)
}

enum AuthStyle {
    Bearer,
}

fn post_json(url: &str, auth: Option<(&str, AuthStyle)>, body: &Value) -> Result<Value, WString> {
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

fn parse_openai_response(response_json: &Value) -> Result<WString, WString> {
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

fn parse_rwkv_batch_response(response_json: &Value) -> Result<Vec<WString>, WString> {
    let Some(choices) = response_json.get("choices").and_then(Value::as_array) else {
        return Err(L!("RWKV Lightning response did not include a choices array.").to_owned());
    };
    if choices.is_empty() {
        return Ok(vec![]);
    }

    let mut slots: Vec<Option<WString>> = vec![None; choices.len()];
    for choice in choices {
        let index = choice
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(0);
        let content = choice
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                L!("RWKV Lightning response did not include choices[*].message.content.").to_owned()
            })?;
        if index >= slots.len() {
            slots.resize(index + 1, None);
        }
        slots[index] = Some(normalize_rwkv_completion(content));
    }

    let mut results = Vec::with_capacity(slots.len());
    for slot in slots {
        let Some(content) = slot else {
            return Err(
                L!("RWKV Lightning response choices were missing one or more indexes.").to_owned(),
            );
        };
        results.push(content);
    }
    Ok(results)
}

fn normalize_rwkv_completion(content: &str) -> WString {
    let mut normalized = content.trim_end_matches(['\n', '\r']).to_owned();
    if let Some(stripped) = normalized.strip_prefix("```fish\n") {
        normalized = stripped.to_owned();
    } else if let Some(stripped) = normalized.strip_prefix("```fish\r\n") {
        normalized = stripped.to_owned();
    } else if let Some(stripped) = normalized.strip_prefix("```\n") {
        normalized = stripped.to_owned();
    }

    normalized = normalized.trim_end_matches(['\n', '\r']).to_owned();
    if let Some(stripped) = normalized.strip_suffix("```") {
        normalized = stripped.trim_end_matches(['\n', '\r']).to_owned();
    }
    trim_response_content(&normalized)
}

fn trim_response_content(content: &str) -> WString {
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

fn parse_provider(provider: Option<WString>, model: &wstr) -> Result<Provider, WString> {
    let provider = provider
        .map(|provider| String::from_utf8_lossy(&wcs2bytes(&provider)).into_owned())
        .unwrap_or_default();
    if !provider.trim().is_empty() {
        return match provider.trim().to_ascii_lowercase().as_str() {
            "openai" | "openai-compatible" => Ok(Provider::OpenAiCompatible),
            "rwkv" | "rwkv-lightning" => Ok(Provider::RwkvLightning),
            _ => Err(sprintf!(
                "Unsupported ANGLER_AI_PROVIDER value: %s",
                provider
            )),
        };
    }

    let model = String::from_utf8_lossy(&wcs2bytes(model)).to_ascii_lowercase();
    if model.starts_with("rwkv") {
        Ok(Provider::RwkvLightning)
    } else {
        Ok(Provider::OpenAiCompatible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rwkv_prompt_ends_with_assistant_fish_prefix() {
        let prompt = rwkv_prompt("查看当前内存占用");
        assert!(prompt.ends_with("Assistant: ```fish\n"));
        assert!(prompt.contains("User: 查看当前内存占用\n\nAssistant: ```fish\n"));
    }

    #[test]
    fn rwkv_completion_strips_trailing_code_fence() {
        assert_eq!(
            normalize_rwkv_completion("free -h\n```\n"),
            L!("free -h").to_owned()
        );
    }

    #[test]
    fn rwkv_completion_strips_echoed_opening_fence() {
        assert_eq!(
            normalize_rwkv_completion("```fish\nfree -h\n```\n"),
            L!("free -h").to_owned()
        );
    }

    #[test]
    fn parse_rwkv_batch_preserves_indexes() {
        let response = json!({
            "choices": [
                {
                    "index": 1,
                    "message": { "content": "docker ps\n```\n" }
                },
                {
                    "index": 0,
                    "message": { "content": "free -h\n```\n" }
                }
            ]
        });
        let results = parse_rwkv_batch_response(&response).unwrap();
        assert_eq!(
            results,
            vec![L!("free -h").to_owned(), L!("docker ps").to_owned()]
        );
    }

    #[test]
    fn provider_defaults_to_rwkv_for_rwkv_models() {
        assert!(matches!(
            parse_provider(None, L!("rwkv7")).unwrap(),
            Provider::RwkvLightning
        ));
    }
}
