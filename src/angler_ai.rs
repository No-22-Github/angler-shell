use crate::prelude::*;
use fish_widestring::{WString, wcs2bytes};

mod context;
mod http;
mod openai;
mod rwkv;

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

    pub(crate) fn chat_completions_url(&self) -> Result<String, WString> {
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
        State::Idle => L!("· AI "),
        State::Loading => L!("⠸ AI "),
        State::Ready(_) => L!("✓ AI "),
        State::Error(_) => L!("! AI "),
    }
}

pub(crate) fn system_prompt() -> String {
    let context = context::ShellContext::capture().as_prompt_section();
    format!(
        "{}\n\n{}",
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
        ),
        context
    )
}

fn request_completion(config: Config, prompt: &wstr) -> Result<WString, WString> {
    match config.provider {
        Provider::OpenAiCompatible => openai::request(config, prompt),
        Provider::RwkvLightning => rwkv::request(config, prompt),
    }
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
    fn provider_defaults_to_rwkv_for_rwkv_models() {
        assert!(matches!(
            parse_provider(None, L!("rwkv7")).unwrap(),
            Provider::RwkvLightning
        ));
    }
}
