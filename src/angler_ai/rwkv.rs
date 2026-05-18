use super::{Config, context::ShellContext, http::post_json, http::trim_response_content};
use crate::prelude::*;
use fish_widestring::{WString, wcs2bytes};
use serde_json::{Value, json};

pub(crate) fn request(config: Config, prompt: &wstr) -> Result<WString, WString> {
    let password = String::from_utf8_lossy(&wcs2bytes(&config.api_key)).into_owned();
    let user_prompt = String::from_utf8_lossy(&wcs2bytes(prompt)).into_owned();
    let url = config.chat_completions_url()?;
    let response = request_batch(&url, &password, &[prompt_for_completion(&user_prompt)])?;
    response.into_iter().next().ok_or_else(|| {
        L!("RWKV Lightning response did not include choices[0].message.content.").to_owned()
    })
}

fn prompt_for_completion(prompt: &str) -> String {
    let context = ShellContext::capture().as_prompt_section();
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
            "{}\n\n",
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
        context, prompt
    )
}

fn request_batch(url: &str, password: &str, prompts: &[String]) -> Result<Vec<WString>, WString> {
    let body = json!({
        "contents": prompts,
        "max_tokens": 1024,
        "stop_tokens": ["```", "\nUser:"],
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
    parse_batch_response(&response_json)
}

fn parse_batch_response(response_json: &Value) -> Result<Vec<WString>, WString> {
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
        slots[index] = Some(normalize_completion(content));
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

fn normalize_completion(content: &str) -> WString {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_ends_with_assistant_fish_prefix() {
        let prompt = prompt_for_completion("查看当前内存占用");
        assert!(prompt.ends_with("Assistant: ```fish\n"));
        assert!(prompt.contains("Environment:\n"));
        assert!(prompt.contains("User: 查看当前内存占用\n\nAssistant: ```fish\n"));
    }

    #[test]
    fn completion_strips_trailing_code_fence() {
        assert_eq!(
            normalize_completion("free -h\n```\n"),
            L!("free -h").to_owned()
        );
    }

    #[test]
    fn completion_strips_echoed_opening_fence() {
        assert_eq!(
            normalize_completion("```fish\nfree -h\n```\n"),
            L!("free -h").to_owned()
        );
    }

    #[test]
    fn parse_batch_preserves_indexes() {
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
        let results = parse_batch_response(&response).unwrap();
        assert_eq!(
            results,
            vec![L!("free -h").to_owned(), L!("docker ps").to_owned()]
        );
    }
}
