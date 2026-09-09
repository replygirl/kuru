//! Explicit opt-in live smoke check; normal tests never contact a model.
use anyhow::{Context, Result, ensure};
use kuru_connectors::{CodexProvider, Provider};
use kuru_core::{CompletionRequest, Message};

#[tokio::main]
async fn main() -> Result<()> {
    let provider = CodexProvider::new("codex");
    let models = provider.models().await?;
    println!("Discovered {} Codex models", models.len());
    for model in &models {
        println!("{}: {}", model.id, model.efforts.join(", "));
    }
    if std::env::args().any(|arg| arg == "--infer") {
        let model = models.first().context("empty Codex model catalog")?;
        let effort = model
            .efforts
            .iter()
            .find(|effort| effort.as_str() == "low")
            .cloned()
            .or_else(|| model.default_effort.clone());
        let result = provider
            .complete(CompletionRequest {
                actor: "live-smoke/isolated".into(),
                instructions:
                    "Return the text OK with no calls. Do not invoke tools or access files.".into(),
                messages: vec![Message {
                    role: "user".into(),
                    content: "Say OK.".into(),
                }],
                model: model.id.clone(),
                effort,
                tools: vec![],
            })
            .await?;
        ensure!(result.calls.is_empty(), "unexpected tool request");
        println!("Live inference: {}", result.text);
    }
    Ok(())
}
