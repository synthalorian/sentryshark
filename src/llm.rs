use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Message,
}

pub struct LlmClient {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
}

impl LlmClient {
    pub fn new(base_url: String, model: String, max_tokens: u32, temperature: f32) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
            max_tokens,
            temperature,
        }
    }

    pub async fn review_code(&self, diff: &str) -> anyhow::Result<String> {
        let prompt = format!(
            "You are a senior code reviewer. Review the following diff and provide:\n\
            1. A summary of changes\n\
            2. Potential bugs or issues\n\
            3. Security concerns\n\
            4. Performance considerations\n\
            5. Suggestions for improvement\n\n\
            Diff:\n```diff\n{}\n```",
            diff
        );

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        let response = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await?;

        let chat_response: ChatResponse = response.json().await?;
        
        Ok(chat_response.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }
}
