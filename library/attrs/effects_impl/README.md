# attrs.effects

- `attrs.effects.direct_passing` produces `{ __flurry_direct_passing: true }` consumed by effect declaration checking for zero-parameter, non-`async` direct-passing effects.
- `attrs.effects.stack_strategy(mode: Symbol)` produces `{ __flurry_effect_strategy: mode }` (`.bounded_pool` | `.guarded_pool` | `.inline_frame`) to declare stack-pool / inlining strategy (`WL-08-02`).
