//! Link lexical instance ports to the declaration's real contextual interface.
//! This runs after every source file has been lowered; it does not type-check
//! or execute code and never resolves interface types in the caller's scope.
use diagnostic::{DiagnosticContext, FlurryError};
use hir::item::TestTemplateInterface;
use hir::{ClauseParamKind, ExprKind, HirArena, ItemKind, Package};

pub(crate) fn link<'hir>(
    package: &mut Package<'hir>,
    arena: &'hir HirArena,
    diagnostics: &DiagnosticContext<'_>,
) {
    let instances = package
        .owners()
        .filter_map(|(owner, info)| {
            matches!(
                info.node.expect_item().kind,
                ItemKind::TestTemplateInstance { .. }
            )
            .then_some(owner)
        })
        .collect::<Vec<_>>();
    for owner in instances {
        let mut item = package.item(owner).unwrap().clone();
        let span = item.span;
        let ItemKind::TestTemplateInstance {
            template,
            interface,
            ports,
            ..
        } = &mut item.kind
        else {
            unreachable!()
        };
        let checked = (|| {
            let mut head = *template;
            while let ExprKind::NFApplication(callee, _) = head.kind {
                head = callee;
            }
            let ExprKind::ResolvedDef { def_id, .. } = head.kind else {
                return Err("test template head must have a declaration identity");
            };
            let definition = package
                .definition_owner(def_id)
                .and_then(|owner| package.item(owner))
                .ok_or("test template declaration is missing from HIR")?;
            let ItemKind::Fn(signature, _) = &definition.kind else {
                return Err("test instance target is not a template definition");
            };
            let parameter = signature
                .test_template_body()?
                .ok_or("test instance target is not a test template")?;
            let mut kind = &parameter.kind;
            while let ClauseParamKind::Quote(inner) = kind {
                kind = inner;
            }
            let ClauseParamKind::Positional(_, ty) = kind else {
                return Err("test template body requires a contextual expression interface");
            };
            let ExprKind::ContextualExprType(formals, _) = &ty.kind else {
                return Err(
                    "test template body interface is not an explicit contextual expression type",
                );
            };
            if ports.len() != formals.len()
                || ports
                    .iter()
                    .zip(formals)
                    .any(|(actual, formal)| actual.name.name != formal.name.name)
            {
                return Err("test template instance ports disagree with its declared interface");
            }
            for (actual, formal) in ports.iter_mut().zip(formals) {
                actual.formal = Some(formal.hir_id);
            }
            *interface = Some(TestTemplateInterface {
                parameter: parameter.hir_id,
                ty,
            });
            Ok(())
        })();
        if let Err(message) = checked {
            crate::LoweringError::malformed_ast(message, span)
                .emit(diagnostics, rustc_span::BytePos(0));
        }
        package.insert_owner(
            owner,
            hir::owner::OwnerInfo {
                node: hir::owner::OwnerNode::Item(arena.alloc_item(item)),
                nodes: hir::owner::OwnerNodes::new(),
            },
        );
    }
}
