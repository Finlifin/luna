//! An optional, bounded pass over original call sites. Expanded bodies are not
//! visited again in this pass, so recursive definitions retain recursive calls.
//! Resource checks and cleanup publication must run after this transformation.
use super::*;
use n::Rewriter;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub expanded: usize,
    pub retained: usize,
}
impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(inline-report (expanded {}) (retained {}))",
            self.expanded, self.retained
        )
    }
}

/// The input is typed NIR. Failed optional expansions leave the original call
/// and metadata intact. Staged calls await input completion; indirect calls and
/// method selection are left to their existing invocation paths.
pub fn expand(package: &mut n::Package, fuel: &mut usize) -> Result<Report, String> {
    let mut templates = BTreeMap::new();
    for function in &package.functions {
        if !function.inline || function.phase != n::FunctionPhase::Runtime {
            continue;
        }
        nir::visit::expressions(function, fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|error| error.to_string())
        })?;
        nir::type_computations::charge_term(&function.signature, fuel)
            .map_err(|e| e.to_string())?;
        templates.insert(function.declaration, function.clone());
    }
    if templates.is_empty() {
        return Ok(Report::default());
    }
    let mut pass = Pass {
        templates: &templates,
        next: &mut package.quote_identity_cursor,
        sources: &mut package.expansion_sources,
        enums: &mut package.enums,
        packets: &mut package.internal_return_packets,
        empty: &mut package.internal_return_empty_replacements,
        fuel,
        report: Report::default(),
    };
    for function in &mut package.functions {
        if function.phase != n::FunctionPhase::Runtime {
            continue;
        }
        // Charge the traversal (including type clones performed by Rewriter)
        // before mutating this body. Exhaustion keeps all remaining bodies.
        if nir::visit::expression(&function.body, pass.fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        })
        .is_err()
        {
            break;
        }
        function.body.rewrite_with(&mut pass)?;
    }
    Ok(pass.report)
}

struct Pass<'a> {
    templates: &'a BTreeMap<DeclarationId, n::Function>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    enums: &'a mut Vec<n::Enum>,
    packets: &'a mut BTreeSet<DeclarationId>,
    empty: &'a mut BTreeMap<(u32, u32), DeclarationId>,
    fuel: &'a mut usize,
    report: Report,
}
impl n::Rewriter for Pass<'_> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        Ok(ty.clone())
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        if *self.fuel == 0 {
            return Ok(false);
        }
        *self.fuel -= 1;
        let K::Call { callee, .. } = &expression.kind else {
            return Ok(true);
        };
        let (id, types) = match &callee.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Ok(true),
        };
        let Some(template) = self.templates.get(&id) else {
            return Ok(true);
        };
        if !matches!(template.signature.callable_signature(),Term::Function {parameters,..}
            if parameters.iter().all(|parameter|parameter.sort==Sort::Runtime))
        {
            self.report.retained += 1;
            return Ok(true);
        }
        // Bound copying before attempting the transaction. Failure is an
        // optimization miss; the already checked ordinary call stays usable.
        let charged = nir::visit::expression(expression, self.fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        });
        if charged.is_err() {
            self.report.retained += 1;
            return Ok(false);
        }
        for ty in types {
            if nir::type_computations::charge_term(ty, self.fuel).is_err() {
                self.report.retained += 1;
                return Ok(false);
            }
        }
        let mut candidate = expression.clone();
        let types = types.to_vec();
        let source_end = self.sources.len();
        let enum_end = self.enums.len();
        let old_next = *self.next;
        let mut packets = BTreeSet::new();
        let mut empty = BTreeMap::new();
        let result = cmtt::inline_template(
            template,
            &mut candidate,
            types,
            self.next,
            self.sources,
            self.enums,
            &mut packets,
            &mut empty,
            self.fuel,
        );
        if result.is_ok() {
            self.packets.extend(packets);
            self.empty.extend(empty);
            *expression = candidate;
            self.report.expanded += 1;
        } else {
            self.sources.truncate(source_end);
            self.enums.truncate(enum_end);
            *self.next = old_next;
            self.report.retained += 1;
        }
        Ok(false)
    }
}
