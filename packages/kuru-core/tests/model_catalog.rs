use kuru_core::{
    FactProvenance, ModelCatalog, ModelInfo, ModelMetadata, ModelRoute, PriceBasis, Sourced,
    advertised_metadata,
};

fn model(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        name: id.into(),
        efforts: vec!["unfamiliar-effort".into()],
        default_effort: Some("unfamiliar-effort".into()),
        metadata: ModelMetadata::default(),
    }
}

#[test]
fn embedded_catalog_enriches_only_matching_live_route_records() {
    let catalog = ModelCatalog::embedded().unwrap();
    let mut live = model("gpt-5.6-sol");
    live.metadata.context_window_tokens = Some(Sourced::advertised(777_777));
    let enriched = catalog.enrich(ModelRoute::OpenAiResponses, live);
    assert_eq!(
        enriched.metadata.context_window_tokens.unwrap().value,
        777_777
    );
    assert_eq!(enriched.metadata.max_output_tokens.unwrap().value, 128_000);
    assert!(matches!(
        enriched.metadata.prices.unwrap().basis,
        PriceBasis::ApiStandard { .. }
    ));
    assert_eq!(enriched.efforts, ["unfamiliar-effort"]);

    let custom = catalog.enrich(ModelRoute::CustomResponses, model("gpt-5.6-sol"));
    assert!(custom.metadata.context_window_tokens.is_none());
    assert!(custom.metadata.prices.is_none());
}

#[test]
fn subscription_context_and_price_basis_are_not_output_or_billing_claims() {
    let model = ModelCatalog::embedded()
        .unwrap()
        .enrich(ModelRoute::CodexSubscription, model("gpt-5.6-sol"));
    assert_eq!(model.metadata.context_window_tokens.unwrap().value, 272_000);
    assert_eq!(
        model.metadata.extended_context_window_tokens.unwrap().value,
        872_000
    );
    assert!(model.metadata.max_output_tokens.is_none());
    assert!(matches!(
        model.metadata.prices.unwrap().basis,
        PriceBasis::ApiEquivalent { .. }
    ));
}

#[test]
fn unknown_models_keep_absent_prices_and_resolve_labelled_fallbacks() {
    let unknown = ModelCatalog::embedded()
        .unwrap()
        .enrich(ModelRoute::OpenAiResponses, model("future-model"));
    assert!(unknown.metadata.prices.is_none());
    assert!(matches!(
        unknown.metadata.resolved_context_window(None).provenance,
        FactProvenance::BuiltInAssumption
    ));
    let configured = unknown.metadata.resolved_context_window(Some(64_000));
    assert_eq!(configured.value, 64_000);
    assert!(matches!(
        configured.provenance,
        FactProvenance::ConfiguredAssumption
    ));

    let known = ModelCatalog::embedded()
        .unwrap()
        .enrich(ModelRoute::OpenAiResponses, model("gpt-5.6-luna"));
    let pinned = known.metadata.resolved_context_window(Some(64_000));
    assert_eq!(pinned.value, 1_050_000);
    assert!(matches!(pinned.provenance, FactProvenance::Pinned { .. }));
}

#[test]
fn malformed_catalogs_are_rejected_before_enrichment() {
    for input in [
        r#"{"schema_version":1,"records":[{"route":"demo","model_id":"x","context_window_tokens":0,"source":{"url":"https://example.test","checked_on":"2026-09-16"}}]}"#,
        r#"{"schema_version":1,"records":[{"route":"demo","model_id":"x","context_window_tokens":10},{"route":"demo","model_id":"x","context_window_tokens":10}]}"#,
        r#"{"schema_version":1,"records":[{"route":"demo","model_id":"x","context_window_tokens":10,"source":{"url":"https://example.test","checked_on":"2026-09-16"},"prices":{"basis":{"api-standard":{"api_model":"x"}},"source":{"url":"https://example.test","checked_on":"2026-09-16"},"input_per_million_usd":"free","output_per_million_usd":"1"}}]}"#,
        r#"{"schema_version":1,"records":[{"route":"open-ai-responses","model_id":"x","prices":{"basis":{"api-equivalent":{"api_model":"x"}},"source":{"url":"https://example.test","checked_on":"2026-09-16"},"input_per_million_usd":"0","output_per_million_usd":"1"}}]}"#,
    ] {
        assert!(ModelCatalog::from_json(input).is_err(), "{input}");
    }
}

#[test]
fn mixed_source_limits_omit_only_the_conflicting_lower_priority_fact() {
    let catalog = ModelCatalog::embedded().unwrap();
    let mut advertised_context = model("gpt-5.6-sol");
    advertised_context.metadata.context_window_tokens = Some(Sourced::advertised(64_000));
    let enriched = catalog.enrich(ModelRoute::OpenAiResponses, advertised_context);
    assert_eq!(
        enriched.metadata.context_window_tokens.unwrap().value,
        64_000
    );
    assert!(enriched.metadata.max_output_tokens.is_none());

    let mut advertised_output = model("gpt-5.6-sol");
    advertised_output.metadata.max_output_tokens = Some(Sourced::advertised(2_000_000));
    let enriched = catalog.enrich(ModelRoute::OpenAiResponses, advertised_output);
    assert!(enriched.metadata.context_window_tokens.is_none());
    assert_eq!(
        enriched.metadata.max_output_tokens.unwrap().value,
        2_000_000
    );

    let advertised = advertised_metadata(Some(64_000), Some(128_000), []);
    assert_eq!(advertised.context_window_tokens.unwrap().value, 64_000);
    assert!(advertised.max_output_tokens.is_none());
}

#[test]
fn sourced_zero_price_is_not_treated_as_absent() {
    let catalog = ModelCatalog::from_json(
        r#"{"schema_version":1,"records":[{"route":"open-ai-responses","model_id":"zero-rate","prices":{"basis":{"api-standard":{"api_model":"zero-rate"}},"source":{"url":"https://example.test","checked_on":"2026-09-16"},"input_per_million_usd":"0","cached_input_per_million_usd":"0","output_per_million_usd":"0"}}]}"#,
    )
    .unwrap();
    let price = catalog
        .enrich(ModelRoute::OpenAiResponses, model("zero-rate"))
        .metadata
        .prices
        .unwrap();
    assert_eq!(price.input_per_million_usd, "0");
    assert_eq!(price.cached_input_per_million_usd.as_deref(), Some("0"));
    assert_eq!(price.output_per_million_usd, "0");
}
