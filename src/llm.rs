use serde::{Deserialize, Serialize};
use tracing::{info, debug};

use crate::config::ReviewConfig;
use crate::inline_comments::{ReviewParser, StructuredReview};

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
    review_config: ReviewConfig,
}

impl LlmClient {
    pub fn new(
        base_url: String,
        model: String,
        max_tokens: u32,
        temperature: f32,
        review_config: ReviewConfig,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
            max_tokens,
            temperature,
            review_config,
        }
    }

    pub async fn review_code(&self,
        diff: &str,
    ) -> anyhow::Result<StructuredReview> {
        let prompt = self.build_prompt(diff);
        debug!("LLM prompt length: {} chars", prompt.len());

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        info!("Sending review request to LLM at {}/v1/chat/completions", self.base_url);

        let response = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await?;

        let chat_response: ChatResponse = response.json().await?;
        
        let content = chat_response.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        info!("Received LLM response: {} chars", content.len());

        let structured = ReviewParser::parse(&content);
        info!(
            "Parsed review: verdict={:?}, {} inline comments",
            structured.verdict,
            structured.inline_comments.len()
        );

        Ok(structured)
    }

    fn build_prompt(&self, diff: &str) -> String {
        let mut prompt = String::new();
        
        prompt.push_str("You are a senior code reviewer. Review the following diff and provide structured feedback.\n\n");
        
        // Add enabled review rules
        prompt.push_str("Focus areas:\n");
        if self.review_config.correctness {
            prompt.push_str("- Correctness: Bugs, logic errors, edge cases, null pointer risks\n");
        }
        if self.review_config.security {
            prompt.push_str("- Security: Injection risks, unsafe operations, secret exposure, authentication/authorization issues\n");
        }
        if self.review_config.performance {
            prompt.push_str("- Performance: Inefficient algorithms, unnecessary allocations, blocking operations, N+1 queries\n");
        }
        if self.review_config.style {
            prompt.push_str("- Style: Code readability, naming, consistency, formatting\n");
        }
        if self.review_config.maintainability {
            prompt.push_str("- Maintainability: Complexity, test coverage, documentation, modularity\n");
        }
        
        prompt.push_str("\n");
        
        // Request structured output
        prompt.push_str("Please provide your review in the following format:\n\n");
        prompt.push_str("VERDICT: [APPROVE | COMMENT | REQUEST_CHANGES]\n\n");
        prompt.push_str("SUMMARY:\n");
        prompt.push_str("[2-3 sentences summarizing the overall assessment]\n\n");
        
        if self.review_config.inline_comments {
            prompt.push_str("For each issue found, include an inline comment:\n\n");
            prompt.push_str("FILE: [file path]\n");
            prompt.push_str("LINE: [line number]\n");
            prompt.push_str("COMMENT: [specific, actionable feedback]\n\n");
            prompt.push_str("Include only inline comments for actual issues - skip praise or minor suggestions.\n\n");
        }
        
        prompt.push_str("Diff:\n```diff\n");
        prompt.push_str(diff);
        prompt.push_str("\n```");
        
        prompt
    }

    pub async fn review_code_simple(&self,
        diff: &str,
    ) -> anyhow::Result<String> {
        let structured = self.review_code(diff).await?;
        Ok(ReviewParser::format_simple_review(&structured))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt() {
        let config = ReviewConfig {
            security: true,
            style: false,
            performance: true,
            correctness: true,
            maintainability: false,
            inline_comments: true,
            summary_comment: true,
        };

        let client = LlmClient::new(
            "http://localhost:8080".to_string(),
            "test".to_string(),
            100,
            0.1,
            config,
        );

        let prompt = client.build_prompt("diff test");
        
        assert!(prompt.contains("Correctness"));
        assert!(prompt.contains("Security"));
        assert!(prompt.contains("Performance"));
        assert!(!prompt.contains("Style"));
        assert!(!prompt.contains("Maintainability"));
        assert!(prompt.contains("VERDICT:"));
        assert!(prompt.contains("FILE:"));
        assert!(prompt.contains("LINE:"));
    }

    #[test]
    fn test_build_prompt_no_inline() {
        let config = ReviewConfig {
            security: false,
            style: false,
            performance: false,
            correctness: true,
            maintainability: false,
            inline_comments: false,
            summary_comment: true,
        };

        let client = LlmClient::new(
            "http://localhost:8080".to_string(),
            "test".to_string(),
            100,
            0.1,
            config,
        );

        let prompt = client.build_prompt("diff test");
        
        assert!(!prompt.contains("FILE:"));
        assert!(!prompt.contains("LINE:"));
        assert!(prompt.contains("VERDICT:"));
    }
}
