use kuru_core::{
    CacheWriteTerms, FactProvenance, ModelCatalog, ModelInfo, ModelMetadata, ModelRoute,
    PriceBasis, Sourced, TokenizerEncoding, advertised_metadata,
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
fn current_sol_and_luna_have_verified_route_scoped_facts() {
    let catalog = ModelCatalog::embedded().unwrap();
    for (id, input, cached, output, writes) in [
        ("gpt-6.1-sol", "2.00", "0.10", "10.00", "2.50"),
        ("gpt-6-luna", "0.10", "0.01", "0.50", "0.125"),
    ] {
        for route in [ModelRoute::OpenAiResponses, ModelRoute::CodexSubscription] {
            let enriched = catalog.enrich(route, model(id));
            assert_eq!(enriched.id, id);
            assert_eq!(enriched.efforts, ["unfamiliar-effort"]);
            assert_eq!(
                enriched.default_effort.as_deref(),
                Some("unfamiliar-effort")
            );
            let prices = enriched.metadata.prices.unwrap();
            assert_eq!(prices.input_per_million_usd, input);
            assert_eq!(prices.cached_input_per_million_usd.as_deref(), Some(cached));
            assert_eq!(prices.output_per_million_usd, output);
            assert_eq!(prices.source.checked_on, "2026-10-08");
            assert!(prices.source.url.ends_with(id));
            assert_eq!(
                prices.cache_write,
                Some(CacheWriteTerms::PerMillionUsd {
                    value: writes.into()
                })
            );
            let tier = prices.long_context_tier.unwrap();
            assert_eq!(tier.input_tokens_over, 272_000);
            assert_eq!(tier.input_multiplier, "2");
            assert_eq!(tier.cached_input_multiplier, "2");
            assert_eq!(tier.output_multiplier, "1.5");
            if route == ModelRoute::OpenAiResponses {
                assert_eq!(
                    enriched.metadata.context_window_tokens.unwrap().value,
                    1_050_000
                );
                assert_eq!(enriched.metadata.max_output_tokens.unwrap().value, 128_000);
                assert_eq!(
                    prices.basis,
                    PriceBasis::ApiStandard {
                        api_model: id.into()
                    }
                );
            } else {
                assert!(enriched.metadata.context_window_tokens.is_none());
                assert!(enriched.metadata.extended_context_window_tokens.is_none());
                assert!(enriched.metadata.max_output_tokens.is_none());
                assert_eq!(
                    prices.basis,
                    PriceBasis::ApiEquivalent {
                        api_model: id.into()
                    }
                );
            }
            assert!(catalog.tokenizer(route, id).is_none());
            let mut live = model(id);
            live.metadata.context_window_tokens = Some(Sourced::advertised(64_000));
            let live = catalog.enrich(route, live);
            assert_eq!(
                live.metadata.context_window_tokens.unwrap(),
                Sourced::advertised(64_000)
            );
            assert!(live.metadata.max_output_tokens.is_none());
        }
        assert_eq!(
            catalog.enrich(ModelRoute::CustomResponses, model(id)),
            model(id)
        );
        // Family shorthand is not a supported alias or metadata match.
        let family = id.rsplit('-').next().unwrap();
        assert!(
            catalog
                .enrich(ModelRoute::OpenAiResponses, model(family))
                .metadata
                .prices
                .is_none()
        );
    }
}

#[test]
fn tokenizer_mapping_is_sourced_and_exactly_route_and_catalog_scoped() {
    let catalog = ModelCatalog::embedded().unwrap();
    for route in [ModelRoute::OpenAiResponses, ModelRoute::CodexSubscription] {
        for id in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            let mapping = catalog.tokenizer(route, id).unwrap();
            assert_eq!(mapping.value, TokenizerEncoding::O200kBase);
            assert!(matches!(mapping.provenance, FactProvenance::Pinned { .. }));
        }
        assert!(catalog.tokenizer(route, "gpt-6-astra").is_none());
        assert!(catalog.tokenizer(route, "gpt-5.6-fictional").is_none());
    }
    assert!(
        catalog
            .tokenizer(ModelRoute::CustomResponses, "gpt-5.6-sol")
            .is_none()
    );
    assert!(
        ModelCatalog::from_json(
            r#"{"schema_version":1,"records":[{"route":"custom-responses","model_id":"gpt-5.6-sol","tokenizer":{"encoding":"o200k_base","source":{"url":"https://example.test","checked_on":"2026-09-22"}}}]}"#
        )
        .is_err()
    );
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
