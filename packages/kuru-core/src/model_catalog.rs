use std::{collections::BTreeSet, sync::OnceLock};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    CacheWriteTerms, LongContextTier, ModelInfo, PriceBasis, PriceSchedule, SourceCitation, Sourced,
};

const EMBEDDED_CATALOG: &str = include_str!("../assets/model-catalog.v1.json");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ModelRoute {
    CodexSubscription,
    OpenAiResponses,
    CustomResponses,
    Demo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelCatalog {
    schema_version: u16,
    records: Vec<CatalogRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogRecord {
    route: ModelRoute,
    model_id: String,
    context_window_tokens: Option<u64>,
    max_output_tokens: Option<u64>,
    extended_context_window_tokens: Option<u64>,
    source: Option<SourceCitation>,
    prices: Option<CatalogPriceSchedule>,
    #[serde(default)]
    capabilities: std::collections::BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogPriceSchedule {
    basis: PriceBasis,
    source: SourceCitation,
    input_per_million_usd: String,
    cached_input_per_million_usd: Option<String>,
    output_per_million_usd: String,
    long_context_tier: Option<LongContextTier>,
    cache_write: Option<CacheWriteTerms>,
    promotional_available_at_least_through: Option<String>,
}

impl ModelCatalog {
    pub fn embedded() -> Result<&'static Self> {
        static CATALOG: OnceLock<Result<ModelCatalog, String>> = OnceLock::new();
        CATALOG
            .get_or_init(|| Self::from_json(EMBEDDED_CATALOG).map_err(|error| error.to_string()))
            .as_ref()
            .map_err(|error| anyhow::Error::msg(error.clone()))
    }

    pub fn from_json(input: &str) -> Result<Self> {
        ensure!(
            input.len() <= 64 * 1024,
            "model catalog asset exceeds 64 KiB"
        );
        let catalog: Self = serde_json::from_str(input).context("parse model catalog")?;
        ensure!(
            catalog.schema_version == 1,
            "unsupported model catalog schema"
        );
        ensure!(
            catalog.records.len() <= 128,
            "model catalog has too many records"
        );
        let mut keys = BTreeSet::new();
        for record in &catalog.records {
            ensure!(
                !record.model_id.trim().is_empty() && record.model_id.len() <= 256,
                "catalog model ID is invalid"
            );
            ensure!(
                keys.insert((record.route, &record.model_id)),
                "duplicate catalog route and model ID"
            );
            let sourced = [
                record.context_window_tokens,
                record.max_output_tokens,
                record.extended_context_window_tokens,
            ];
            if sourced.iter().any(Option::is_some) || !record.capabilities.is_empty() {
                validate_citation(
                    record
                        .source
                        .as_ref()
                        .context("catalog fact lacks source")?,
                )?;
            }
            for limit in sourced.into_iter().flatten() {
                ensure!(limit > 0, "catalog token limits must be positive");
            }
            if let (Some(context), Some(output)) =
                (record.context_window_tokens, record.max_output_tokens)
            {
                ensure!(
                    output <= context,
                    "catalog output limit exceeds context window"
                );
            }
            for capability in record.capabilities.keys() {
                ensure!(
                    !capability.trim().is_empty() && capability.len() <= 128,
                    "catalog capability name is invalid"
                );
            }
            if let Some(prices) = &record.prices {
                ensure!(
                    matches!(
                        (record.route, &prices.basis),
                        (
                            ModelRoute::CodexSubscription,
                            PriceBasis::ApiEquivalent { .. }
                        ) | (ModelRoute::OpenAiResponses, PriceBasis::ApiStandard { .. })
                    ),
                    "catalog route and price basis do not match"
                );
                validate_prices(prices)?;
            }
        }
        Ok(catalog)
    }

    pub fn enrich(&self, route: ModelRoute, mut model: ModelInfo) -> ModelInfo {
        let Some(record) = self
            .records
            .iter()
            .find(|record| record.route == route && record.model_id == model.id)
        else {
            return model;
        };
        let citation = record.source.clone();
        fill_compatible_limits(
            &mut model.metadata.context_window_tokens,
            &mut model.metadata.max_output_tokens,
            record.context_window_tokens,
            record.max_output_tokens,
            citation.clone(),
        );
        fill_sourced(
            &mut model.metadata.extended_context_window_tokens,
            record.extended_context_window_tokens,
            citation,
        );
        for (name, value) in &record.capabilities {
            model
                .metadata
                .capabilities
                .entry(name.clone())
                .or_insert_with(|| Sourced::pinned(*value, record.source.clone().unwrap()));
        }
        if model.metadata.prices.is_none() {
            model.metadata.prices = record.prices.as_ref().map(|prices| PriceSchedule {
                basis: prices.basis.clone(),
                source: prices.source.clone(),
                input_per_million_usd: prices.input_per_million_usd.clone(),
                cached_input_per_million_usd: prices.cached_input_per_million_usd.clone(),
                output_per_million_usd: prices.output_per_million_usd.clone(),
                long_context_tier: prices.long_context_tier.clone(),
                cache_write: prices.cache_write.clone(),
                promotional_available_at_least_through: prices
                    .promotional_available_at_least_through
                    .clone(),
            });
        }
        model
    }
}

pub fn enrich_model(route: ModelRoute, model: ModelInfo) -> Result<ModelInfo> {
    Ok(ModelCatalog::embedded()?.enrich(route, model))
}

fn fill_compatible_limits(
    context: &mut Option<Sourced<u64>>,
    output: &mut Option<Sourced<u64>>,
    snapshot_context: Option<u64>,
    snapshot_output: Option<u64>,
    citation: Option<SourceCitation>,
) {
    if context.is_none()
        && let (Some(value), Some(citation)) = (snapshot_context, citation.clone())
        && output.as_ref().is_none_or(|output| output.value <= value)
    {
        *context = Some(Sourced::pinned(value, citation));
    }
    if output.is_none()
        && let (Some(value), Some(citation)) = (snapshot_output, citation)
        && context
            .as_ref()
            .is_none_or(|context| value <= context.value)
    {
        *output = Some(Sourced::pinned(value, citation));
    }
}

fn fill_sourced(
    target: &mut Option<Sourced<u64>>,
    value: Option<u64>,
    citation: Option<SourceCitation>,
) {
    if target.is_none()
        && let (Some(value), Some(citation)) = (value, citation)
    {
        *target = Some(Sourced::pinned(value, citation));
    }
}

fn validate_citation(citation: &SourceCitation) -> Result<()> {
    ensure!(
        citation.url.starts_with("https://") && citation.url.len() <= 1024,
        "catalog source URL is invalid"
    );
    ensure!(
        citation.checked_on.len() == 10
            && citation
                .checked_on
                .bytes()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) && byte == b'-'
                    || !matches!(index, 4 | 7) && byte.is_ascii_digit()),
        "catalog source checked date is invalid"
    );
    Ok(())
}

fn validate_prices(prices: &CatalogPriceSchedule) -> Result<()> {
    validate_citation(&prices.source)?;
    validate_nonnegative_decimal(&prices.input_per_million_usd)?;
    validate_nonnegative_decimal(&prices.output_per_million_usd)?;
    if let Some(value) = &prices.cached_input_per_million_usd {
        validate_nonnegative_decimal(value)?;
    }
    if let Some(tier) = &prices.long_context_tier {
        ensure!(
            tier.input_tokens_over > 0,
            "catalog price tier threshold must be positive"
        );
        validate_positive_decimal(&tier.input_multiplier)?;
        validate_positive_decimal(&tier.cached_input_multiplier)?;
        validate_positive_decimal(&tier.output_multiplier)?;
    }
    if let Some(cache_write) = &prices.cache_write {
        match cache_write {
            CacheWriteTerms::PerMillionUsd { value } => validate_nonnegative_decimal(value)?,
            CacheWriteTerms::InputMultiplier { value } => validate_positive_decimal(value)?,
        }
    }
    if let Some(date) = &prices.promotional_available_at_least_through {
        validate_citation(&SourceCitation {
            url: prices.source.url.clone(),
            checked_on: date.clone(),
        })?;
    }
    match &prices.basis {
        PriceBasis::ApiStandard { api_model } | PriceBasis::ApiEquivalent { api_model } => {
            ensure!(
                !api_model.trim().is_empty(),
                "catalog API price basis lacks model ID"
            );
        }
    }
    Ok(())
}

fn validate_nonnegative_decimal(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && value.bytes().filter(|byte| *byte == b'.').count() <= 1
            && value.bytes().any(|byte| byte.is_ascii_digit())
            && value
                .parse::<f64>()
                .is_ok_and(|number| number >= 0.0 && number.is_finite()),
        "catalog price is not a nonnegative decimal"
    );
    Ok(())
}

fn validate_positive_decimal(value: &str) -> Result<()> {
    validate_nonnegative_decimal(value)?;
    ensure!(
        value.parse::<f64>().is_ok_and(|number| number > 0.0),
        "catalog multiplier is not a positive decimal"
    );
    Ok(())
}

pub fn advertised_metadata(
    context_window_tokens: Option<u64>,
    max_output_tokens: Option<u64>,
    capabilities: impl IntoIterator<Item = (String, bool)>,
) -> crate::ModelMetadata {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0);
    let max_output_tokens = max_output_tokens
        .filter(|value| *value > 0)
        .filter(|output| context_window_tokens.is_none_or(|context| *output <= context));
    crate::ModelMetadata {
        context_window_tokens: context_window_tokens.map(Sourced::advertised),
        max_output_tokens: max_output_tokens.map(Sourced::advertised),
        capabilities: capabilities
            .into_iter()
            .filter(|(name, _)| !name.trim().is_empty() && name.len() <= 128)
            .map(|(name, value)| (name, Sourced::advertised(value)))
            .collect(),
        ..crate::ModelMetadata::default()
    }
}
