//! Check the source-selected push iteration contract before lowering consumers.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn validate_iterator_protocol(&self, id: DeclarationId) -> Result<(), Error> {
        let span = rustc_span::DUMMY_SP;
        let fail = |message: &str| error(span, message);
        let mut candidates = self
            .checked_traits
            .iter()
            .filter(|item| item.declaration == id);
        let interface = candidates
            .next()
            .ok_or_else(|| fail("Iterator binding is not a trait"))?;
        if candidates.next().is_some() || !interface.type_parameters.is_empty() {
            return Err(fail(
                "Iterator requires one trait identity without explicit parameters",
            ));
        }
        let mut items = interface
            .associated
            .iter()
            .filter(|item| item.name == "Item");
        let item = items
            .next()
            .ok_or_else(|| fail("Iterator requires an associated Item"))?;
        let mut fuel = 100_000;
        nir::type_computations::charge_term(&item.ty, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        if items.next().is_some() || item.ty != p(P::Type) {
            return Err(fail("Iterator requires one Type-valued Item"));
        }
        let mut methods = interface
            .methods
            .iter()
            .filter(|method| method.name == "iterate");
        let method = methods
            .next()
            .ok_or_else(|| fail("Iterator requires an iterate method"))?;
        if methods.next().is_some()
            || !method.by_value_receiver
            || method.by_reference_receiver
            || !method.type_parameters.is_empty()
            || !method.type_arguments.is_empty()
            || method.signature.is_comptime_callable()
            || method.is_quote
            || method.parameter_bindings.len() != 1
        {
            return Err(fail(
                "Iterator requires runtime fn iterate(self) without method parameters",
            ));
        }
        let signature = method
            .signature
            .callable_signature()
            .map_err(|_| fail("Iterator iterate must be callable"))?;
        nir::type_computations::charge_term(signature, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        let subject = Term::Rigid {
            declaration: interface.self_parameter,
            sort: Sort::Type,
        };
        let Term::Function { parameters, result } = signature else {
            return Err(fail("Iterator iterate must be callable"));
        };
        if parameters.as_slice()
            != [nir::Parameter {
                sort: Sort::Runtime,
                ty: subject.clone(),
            }]
        {
            return Err(fail(
                "Iterator iterate requires exactly one by-value Self receiver",
            ));
        }
        let Term::Computation { effects, result } = result.as_ref() else {
            return Err(fail(
                "Iterator iterate must return an element-delivery computation",
            ));
        };
        let [operation] = effects.as_slice() else {
            return Err(fail(
                "Iterator iterate requires exactly one element-delivery operation",
            ));
        };
        if **result != p(P::Void) {
            return Err(fail("Iterator iterate computation must return void"));
        }
        let metadata = self.effect_metadata(operation, span)?;
        if !metadata.inline || metadata.abi != nir::effects::Abi::Direct {
            return Err(fail(
                "Iterator element delivery requires a direct inline effect",
            ));
        }
        let signature = self.formed_effect_signature(&metadata, operation, span)?;
        let element = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id,
                member: item.declaration,
            }),
            arguments: vec![],
            subject: Box::new(subject),
        };
        let expected = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: element,
            }],
            result: Box::new(p(P::Void)),
        };
        if signature != expected {
            return Err(fail(
                "Iterator element delivery requires one Item argument and a void result",
            ));
        }
        Ok(())
    }
}
