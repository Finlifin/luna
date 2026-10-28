/// Supply the real attrs provider to an isolated builtin module fixture.
#[allow(dead_code)]
pub(crate) fn with_attrs(source: &str) -> String {
    format!(
        "mod attrs {{ {} }}\n{source}",
        include_str!("../../../../library/attrs/lib.fl")
    )
}
