//! Immutable UTF-8 literals. No allocator or mutable string API is implied.
use super::*;

/// Module-wide storage identities, shared by named bodies, closures and
/// outlined handlers. The logical String/Symbol type stays on the Value;
/// sharing immutable bytes does not equate those types or establish provenance.
#[derive(Default)]
pub(super) struct TextConstants {
    entries: std::collections::HashMap<String, usize>,
}
impl TextConstants {
    fn intern(
        &mut self,
        text: &str,
        declarations: &mut BTreeSet<String>,
        fuel: &mut usize,
    ) -> Result<String, String> {
        // A hit still hashes and compares all bytes. Charge before lookup,
        // including empty strings, so cached data never bypasses the budget.
        let work = text.len().checked_add(1).ok_or("host text size overflow")?;
        *fuel = fuel
            .checked_sub(work)
            .ok_or("host text lookup budget exhausted")?;
        if let Some(index) = self.entries.get(text) {
            return Ok(format!("flurry_string_{index}"));
        }
        // Assign by first use, never HashMap iteration order. Publish only
        // after encoding succeeds, leaving failed requests retryable.
        let index = self.entries.len();
        let symbol = format!("flurry_string_{index}");
        let declaration = text_constant(text, &symbol, fuel)?;
        declarations.insert(declaration);
        self.entries.insert(text.to_owned(), index);
        Ok(symbol)
    }
}
impl Emitter<'_> {
    pub(super) fn literal(&mut self, literal: &Literal, ty: Ty) -> Result<Value, String> {
        let text = match (literal, &ty) {
            (Literal::String(text), Ty::String) | (Literal::Symbol(text), Ty::Symbol) => text,
            (Literal::String(_), _) | (Literal::Symbol(_), _) => {
                return Err("host text literal representation mismatch".into());
            }
            _ => return literal_value(literal, ty),
        };
        self.text_literal(text, ty)
    }
    pub(super) fn text_literal(&mut self, text: &str, ty: Ty) -> Result<Value, String> {
        if !matches!(ty, Ty::String | Ty::Symbol) {
            return Err("host text literal representation mismatch".into());
        }
        let symbol = self
            .text_constants
            .intern(text, self.declarations, self.fuel)?;
        Ok(Value {
            ty,
            text: format!("{{ ptr @{symbol}, i64 {} }}", text.len()),
        })
    }
    pub(super) fn string_equal(&mut self, left: &Value, right: &Value) -> Value {
        self.declarations
            .insert(include_str!("../string_equal.ll").into());
        self.instruction(
            Ty::Bool,
            format!(
                "call i1 @flurry_string_equal({{ ptr, i64 }} {}, {{ ptr, i64 }} {})",
                left.text, right.text
            ),
        )
    }
}

/// Charge the bytes before encoding or publishing a global. Every source byte
/// becomes one LLVM hexadecimal escape; no work is hidden behind a single node.
fn text_constant(text: &str, symbol: &str, fuel: &mut usize) -> Result<String, String> {
    use std::fmt::Write;
    let work = text.len().checked_add(1).ok_or("host text size overflow")?;
    *fuel = fuel
        .checked_sub(work)
        .ok_or("host text literal budget exhausted")?;
    let mut declaration = format!(
        "@{symbol} = private unnamed_addr constant [{} x i8] c\"",
        text.len()
    );
    for byte in text.bytes() {
        write!(&mut declaration, "\\{byte:02X}").expect("writing to a String cannot fail");
    }
    declaration.push('"');
    Ok(declaration)
}

#[cfg(test)]
mod tests {
    use super::{BTreeSet, TextConstants, text_constant};

    #[test]
    fn text_pool_preserves_bytes_identity_and_failed_publication() {
        let mut pool = TextConstants::default();
        let mut declarations = BTreeSet::new();
        let text = "中\0\"";
        let work = text.len() + 1;
        // Lookup succeeds, encoding fails: no orphan global or cache entry.
        assert!(pool.intern(text, &mut declarations, &mut { work }).is_err());
        assert!(pool.entries.is_empty());
        assert!(declarations.is_empty());
        let first = pool
            .intern(text, &mut declarations, &mut (2 * work))
            .unwrap();
        assert_eq!(first, "flurry_string_0");
        assert_eq!(
            pool.intern(text, &mut declarations, &mut { work }).unwrap(),
            first
        );
        assert_eq!(declarations.len(), 1);
        assert!(
            pool.intern(text, &mut declarations, &mut (work - 1))
                .is_err()
        );
        // Unrelated declarations cannot change storage naming or cause aliases.
        declarations.insert("declare void @unrelated()".into());
        assert_eq!(
            pool.intern("", &mut declarations, &mut 2).unwrap(),
            "flurry_string_1"
        );
        assert_eq!(
            pool.intern("中", &mut declarations, &mut 8).unwrap(),
            "flurry_string_2"
        );
        assert_eq!(pool.entries.len(), 3);
        assert_eq!(declarations.len(), 4);
    }

    #[test]
    fn text_encoding_charges_utf8_bytes_before_allocation() {
        let text = "a\0\"\\\n中";
        let mut fuel = text.len();
        assert_eq!(
            text_constant(text, "test", &mut fuel).unwrap_err(),
            "host text literal budget exhausted"
        );
        assert_eq!(fuel, text.len());
        let mut fuel = text.len() + 1;
        let declaration = text_constant(text, "test", &mut fuel).unwrap();
        assert_eq!(fuel, 0);
        assert_eq!(
            declaration,
            "@test = private unnamed_addr constant [8 x i8] c\"\\61\\00\\22\\5C\\0A\\E4\\B8\\AD\""
        );
        assert!(text_constant(text, "test", &mut fuel).is_err());
        let mut fresh = text.len() + 1;
        assert_eq!(
            text_constant(text, "test", &mut fresh).unwrap(),
            declaration
        );
    }

    #[test]
    fn empty_text_still_charges_for_its_global_declaration() {
        assert!(text_constant("", "empty", &mut 0).is_err());
        assert_eq!(
            text_constant("", "empty", &mut 1).unwrap(),
            "@empty = private unnamed_addr constant [0 x i8] c\"\""
        );
    }
}
