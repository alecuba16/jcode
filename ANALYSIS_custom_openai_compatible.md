# Custom OpenAI-Compatible Provider Handling: jcode vs opencode Parity Analysis

## 1. How custom OpenAI-compatible models are loaded into jcode

### Configuration source
- **config.toml** `[providers.<name>]` → `NamedProviderConfig` struct (`crates/jcode-config-types/src/lib.rs:431`)
- Built-in profiles → `OpenAiCompatibleProfile` catalog (`crates/jcode-provider-metadata/src/catalog.rs`)
- CLI/TUI command `jcode login openai-compatible` / `--provider-profile <name>`

### Loading flow
1. `apply_named_provider_profile_env_from_config()` (`provider_catalog.rs:645`) sets env vars (`JCODE_OPENROUTER_API_BASE`, `JCODE_NAMED_PROVIDER_PROFILE`, static models, auth header mode)
2. `OpenRouterProvider::new_named_openai_compatible()` (`openrouter-runtime/src/lib.rs:1330`) constructs the runtime instance reading: `base_url`, `auth`, `api_key_env`, `default_model`, `models[]`, `extra_body`, `supports_reasoning_effort`
3. Catalog routes built by `named_provider_profile_routes()` (`catalog_routes.rs:493`) → iterates `models[]`, filters by `input` capability, builds `ModelRoute` with `provider = profile_name`, `api_method = "openai-compatible:<profile>"`, `detail = base_url`

### /model list population
- `append_direct_openai_compatible_profile_routes()` (`catalog_routes.rs:460`) adds routes for **all** named profiles from config (even if not active) → **custom models always appear in `/model` list**
- Active profile models come through the OpenRouter slot with live catalog freshness
- Non-active profile models appear as static routes with `available: true`

## 2. Reasoning effort handling

### Current state
- **Provider-level** `supports_reasoning_effort: Option<bool>` (`NamedProviderConfig:469`) — explicit override, `None` = auto-detect
- Auto-detection: DeepSeek profile id or DeepSeek-family model name → DeepSeek-style effort; GPT-family model → OpenAI-style effort
- **Per-model reasoning support: NONE** — no `reasoning` field on `NamedProviderModelConfig`
- Available effort ladder is provider-wide (`available_efforts()` in `openrouter_provider_impl.rs:471`)

### Parity gap
- opencode has per-model `reasoning: bool` in its schema
- jcode only has provider-level `supports_reasoning_effort`

## 3. Parity comparison with opencode

### opencode config schema (from `https://opencode.ai/config.json`)

```jsonc
"provider": {
  "myprovider": {
    "npm": "@ai-sdk/openai-compatible",   // ← jcode: hardcoded (always openai-compatible)
    "name": "My Display Name",            // ← jcode: MISSING (uses profile key)
    "options": { "baseURL": "...", "apiKey": "...", "headers": {...} },
    "models": {
      "model-id": {
        "name": "Model Display Name",     // ← jcode: MISSING
        "reasoning": true,                // ← jcode: MISSING (provider-level only)
        "cost": {                         // ← jcode: MISSING
          "input": 2.0,
          "output": 8.0,
          "cache_read": 0.5,
          "cache_write": 2.0
        },
        "limit": {
          "context": 200000,              // ← jcode: has context_window
          "output": 65536                // ← jcode: MISSING
        },
        "modalities": { "input": ["text","image"], "output": ["text"] },  // jcode: has input[]
        "options": { "reasoningEffort": "high" },  // ← jcode: provider-level only
        "variants": { ... }               // ← jcode: MISSING (swarm-only)
      }
    }
  }
}
```

### Parity gaps table

| Feature | opencode | jcode | Status |
|---------|----------|-------|--------|
| Provider display name in picker | `name` field | uses profile key (e.g. `my-gateway`) | **GAP** |
| Model display name/alias in picker | `name` field | uses model `id` | **GAP** |
| Per-model cost (input/output/cache_read/cache_write) | `cost` object | not configurable (models.dev only) | **GAP** |
| Per-model max output tokens | `limit.output` | not configurable | **GAP** |
| Per-model reasoning support | `reasoning: bool` | provider-level only | **GAP** |
| Per-model reasoning effort default | `options.reasoningEffort` | provider-level only | **MINOR GAP** |
| Per-model custom headers | `headers` object | not configurable | **MINOR GAP** |
| Per-model modalities | `modalities` object | `input: Vec<String>` | **PARTIAL** |
| Context window | `limit.context` | `context_window` | ✅ |
| Provider base URL | `options.baseURL` | `base_url` | ✅ |
| API key env | `apiKey` / env | `api_key_env` / `api_key` | ✅ |
| Auth method | implicit | `auth` (Bearer/Header/None) | ✅ (jcode richer) |
| Extra request body | not standard | `extra_body` | ✅ (jcode richer) |
| Reasoning effort support flag | `reasoning: true` + options | `supports_reasoning_effort` | ✅ (provider-level) |

## 4. Plan

### Phase 1: Config schema enrichment (jcode-config-types)

**`NamedProviderConfig`** — add:
- `display_name: Option<String>` — provider display name shown in picker

**`NamedProviderModelConfig`** — add:
- `display_name: Option<String>` (alias `name`) — model alias shown in picker
- `reasoning: Option<bool>` — per-model reasoning support flag
- `max_output_tokens: Option<usize>` (alias via `limit.output`) — max output tokens
- `cost: Option<ModelCostConfig>` — per-model pricing
  - `input_usd_per_mtok: Option<f64>`
  - `output_usd_per_mtok: Option<f64>`
  - `cache_read_usd_per_mtok: Option<f64>`
  - `cache_write_usd_per_mtok: Option<f64>`

### Phase 2: Wiring through the stack

**catalog_routes.rs** `named_provider_profile_routes()`:
- Use `display_name` from `NamedProviderConfig` as `ModelRoute.provider` (fallback to profile key)
- Attach `ModelRoute.cheapness` from configured `cost`

**OpenRouter runtime** (`new_named_openai_compatible`):
- Store `display_name` and per-model `display_name`/`reasoning`/`max_output_tokens`/`cost`
- Use per-model `display_name` in `available_models_display()` 
- Use per-model `reasoning` in `available_efforts()` / `supports_*_reasoning_effort()`
- Use `max_output_tokens` in request building

**model_pricing.rs / RouteCheapnessEstimate**:
- User-configured cost overrides models.dev catalog for custom models

### Phase 3: Tests + docs

- Unit tests for new config fields deserialization
- Test route building with display_name + cost
- Update default config.toml with documented examples