//! Deterministic ordering wrappers for NIR terms and sort frames in phase storage tables.
use crate::{Parameter, Sort, Term, effects};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct OrderedSort(pub(super) Sort);

impl Ord for OrderedSort {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_sort(self.0, other.0)
    }
}

impl PartialOrd for OrderedSort {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OrderedTerm(pub Term);

impl Ord for OrderedTerm {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_term(&self.0, &other.0)
    }
}

impl PartialOrd for OrderedTerm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(super) fn ordered_frames(frames: &[Vec<Sort>]) -> Vec<Vec<OrderedSort>> {
    frames
        .iter()
        .map(|frame| frame.iter().copied().map(OrderedSort).collect())
        .collect()
}

fn cmp_slice_by<T>(a: &[T], b: &[T], mut cmp: impl FnMut(&T, &T) -> Ordering) -> Ordering {
    for (left, right) in a.iter().zip(b.iter()) {
        let ord = cmp(left, right);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

fn cmp_sort(a: Sort, b: Sort) -> Ordering {
    let rank = |s: Sort| match s {
        Sort::Type => 0u8,
        Sort::ComptimeValue => 1,
        Sort::Runtime => 2,
    };
    rank(a).cmp(&rank(b))
}

pub fn cmp_effect_operation(a: &effects::Operation, b: &effects::Operation) -> Ordering {
    a.declaration
        .cmp(&b.declaration)
        .then_with(|| cmp_term_slice(&a.arguments, &b.arguments))
}

fn cmp_parameter(a: &Parameter, b: &Parameter) -> Ordering {
    cmp_sort(a.sort, b.sort).then_with(|| cmp_term(&a.ty, &b.ty))
}

fn cmp_record_fields(a: &[(String, Term)], b: &[(String, Term)]) -> Ordering {
    cmp_slice_by(a, b, |(an, at), (bn, bt)| {
        an.cmp(bn).then_with(|| cmp_term(at, bt))
    })
}

fn cmp_option_box_term(a: &Option<Box<Term>>, b: &Option<Box<Term>>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => cmp_term(a, b),
    }
}

fn term_rank(term: &Term) -> u8 {
    match term {
        Term::Primitive(_) => 0,
        Term::ErrorResult { .. } => 1,
        Term::Computation { .. } => 2,
        Term::AssociatedType { .. } => 3,
        Term::Nominal { .. } => 4,
        Term::TraitObject { .. } => 5,
        Term::NormalForm { .. } => 6,
        Term::ClosureContextType => 7,
        Term::EffectRowType => 8,
        Term::ErrorRowType => 9,
        Term::ReachabilityType => 10,
        Term::AnyType => 11,
        Term::ClosureContextValue(_) => 12,
        Term::ContextualClosure { .. } => 13,
        Term::Closure { .. } => 14,
        Term::Tuple(_) => 15,
        Term::Record(_) => 16,
        Term::TupleValue(_) => 17,
        Term::ListConcat { .. } => 18,
        Term::RecordValue(_) => 19,
        Term::NominalValue { .. } => 20,
        Term::OptionalValue { .. } => 21,
        Term::ErrorValue { .. } => 22,
        Term::Optional(_) => 23,
        Term::Pointer(_) => 24,
        Term::Lift(_) => 25,
        Term::ComptimeFunction(_) => 26,
        Term::NormalFormFunction(_, _) => 27,
        Term::UnsafeFunction(_) => 28,
        Term::Function { .. } => 29,
        Term::Hole { .. } => 30,
        Term::Bound { .. } => 31,
        Term::Rigid { .. } => 32,
        Term::Runtime(_) => 33,
        Term::RuntimeField { .. } => 34,
        Term::Unit => 35,
        Term::Bool(_) => 36,
        Term::Char(_) => 37,
        Term::BitVec(_) => 38,
        Term::Float16(_) => 39,
        Term::BFloat16(_) => 40,
        Term::Float32(_) => 41,
        Term::Float64(_) => 42,
        Term::String(_) => 43,
        Term::Symbol(_) => 44,
        Term::MethodValue(_) => 45,
        Term::FunctionValue { .. } => 46,
        Term::ClosureSnapshot { .. } => 47,
        Term::ClosureValue { .. } => 48,
        Term::Integer(_) => 49,
        Term::Float(_) => 50,
        Term::Value { .. } => 51,
        Term::PureFunction(_) => 52,
        Term::AssociatedTypeCall(_) => 53,
        Term::ComptimeCall(_) => 54,
        Term::DeferredErrorResult { .. } => 55,
        Term::FieldType { .. } => 56,
        Term::VariantFieldType { .. } => 57,
        Term::TupleAppend { .. } => 58,
    }
}

pub fn cmp_term_slice(a: &[Term], b: &[Term]) -> Ordering {
    cmp_slice_by(a, b, cmp_term)
}

pub fn cmp_term(a: &Term, b: &Term) -> Ordering {
    let ord = term_rank(a).cmp(&term_rank(b));
    if ord != Ordering::Equal {
        return ord;
    }
    match (a, b) {
        (Term::Primitive(a), Term::Primitive(b)) => a.cmp(b),
        (
            Term::ErrorResult {
                errors: ae,
                result: ar,
            },
            Term::ErrorResult {
                errors: be,
                result: br,
            },
        ) => cmp_term_slice(ae, be).then_with(|| cmp_term(ar, br)),
        (
            Term::Computation {
                effects: ae,
                result: ar,
            },
            Term::Computation {
                effects: be,
                result: br,
            },
        ) => cmp_slice_by(ae, be, cmp_effect_operation).then_with(|| cmp_term(ar, br)),
        (
            Term::AssociatedType {
                origin: ao,
                arguments: aa,
                subject: asub,
            },
            Term::AssociatedType {
                origin: bo,
                arguments: ba,
                subject: bsub,
            },
        ) => ao
            .interface
            .cmp(&bo.interface)
            .then_with(|| ao.member.cmp(&bo.member))
            .then_with(|| cmp_term_slice(aa, ba))
            .then_with(|| cmp_term(asub, bsub)),
        (Term::AssociatedTypeCall(a), Term::AssociatedTypeCall(b)) => a
            .origin
            .interface
            .cmp(&b.origin.interface)
            .then_with(|| a.origin.member.cmp(&b.origin.member))
            .then_with(|| cmp_term_slice(&a.arguments, &b.arguments))
            .then_with(|| cmp_term(&a.subject, &b.subject))
            .then_with(|| cmp_term_slice(&a.call_arguments, &b.call_arguments)),
        (Term::ComptimeCall(a), Term::ComptimeCall(b)) => a
            .declaration
            .cmp(&b.declaration)
            .then_with(|| cmp_term_slice(&a.type_arguments, &b.type_arguments))
            .then_with(|| cmp_term_slice(&a.arguments, &b.arguments))
            .then_with(|| cmp_term(&a.ty, &b.ty)),
        (
            Term::DeferredErrorResult {
                errors: ae,
                result: ar,
            },
            Term::DeferredErrorResult {
                errors: be,
                result: br,
            },
        ) => cmp_term(ae, be).then_with(|| cmp_term(ar, br)),
        (
            Term::FieldType {
                subject: asub,
                field: af,
            },
            Term::FieldType {
                subject: bsub,
                field: bf,
            },
        ) => cmp_term(asub, bsub).then_with(|| cmp_term(af, bf)),
        (
            Term::VariantFieldType {
                subject: asub,
                variant: av,
                field: af,
            },
            Term::VariantFieldType {
                subject: bsub,
                variant: bv,
                field: bf,
            },
        ) => cmp_term(asub, bsub)
            .then_with(|| cmp_term(av, bv))
            .then_with(|| cmp_term(af, bf)),
        (
            Term::TupleAppend {
                tuple: at,
                element: ae,
            },
            Term::TupleAppend {
                tuple: bt,
                element: be,
            },
        ) => cmp_term(at, bt).then_with(|| cmp_term(ae, be)),
        (
            Term::Nominal {
                declaration: ad,
                arguments: aa,
            },
            Term::Nominal {
                declaration: bd,
                arguments: ba,
            },
        ) => ad.cmp(bd).then_with(|| cmp_term_slice(aa, ba)),
        (
            Term::TraitObject {
                interface: ai,
                arguments: aa,
            },
            Term::TraitObject {
                interface: bi,
                arguments: ba,
            },
        ) => ai.cmp(bi).then_with(|| cmp_term_slice(aa, ba)),
        (
            Term::NormalForm {
                head: ah,
                sort: asort,
                arguments: aa,
            },
            Term::NormalForm {
                head: bh,
                sort: bsort,
                arguments: ba,
            },
        ) => ah
            .cmp(bh)
            .then_with(|| cmp_sort(*asort, *bsort))
            .then_with(|| cmp_term_slice(aa, ba)),
        (Term::ClosureContextType, Term::ClosureContextType)
        | (Term::EffectRowType, Term::EffectRowType)
        | (Term::ErrorRowType, Term::ErrorRowType)
        | (Term::ReachabilityType, Term::ReachabilityType)
        | (Term::AnyType, Term::AnyType)
        | (Term::Unit, Term::Unit) => Ordering::Equal,
        (Term::ClosureContextValue(a), Term::ClosureContextValue(b))
        | (Term::Tuple(a), Term::Tuple(b))
        | (Term::TupleValue(a), Term::TupleValue(b)) => cmp_term_slice(a, b),
        (
            Term::ContextualClosure {
                context: ac,
                signature: asig,
            },
            Term::ContextualClosure {
                context: bc,
                signature: bsig,
            },
        ) => cmp_term(ac, bc).then_with(|| cmp_term(asig, bsig)),
        (
            Term::Closure {
                environment: ae,
                signature: asig,
            },
            Term::Closure {
                environment: be,
                signature: bsig,
            },
        ) => cmp_term_slice(ae, be).then_with(|| cmp_term(asig, bsig)),
        (Term::Record(a), Term::Record(b)) | (Term::RecordValue(a), Term::RecordValue(b)) => {
            cmp_record_fields(a, b)
        }
        (Term::ListConcat { ty: at, parts: ap }, Term::ListConcat { ty: bt, parts: bp }) => {
            cmp_term(at, bt).then_with(|| cmp_term_slice(ap, bp))
        }
        (
            Term::NominalValue {
                ty: at,
                variant: av,
                fields: af,
            },
            Term::NominalValue {
                ty: bt,
                variant: bv,
                fields: bf,
            },
        ) => cmp_term(at, bt)
            .then_with(|| av.cmp(bv))
            .then_with(|| cmp_term_slice(af, bf)),
        (Term::OptionalValue { ty: at, value: av }, Term::OptionalValue { ty: bt, value: bv }) => {
            cmp_term(at, bt).then_with(|| cmp_option_box_term(av, bv))
        }
        (
            Term::ErrorValue {
                ty: at,
                label: al,
                value: av,
            },
            Term::ErrorValue {
                ty: bt,
                label: bl,
                value: bv,
            },
        ) => cmp_term(at, bt)
            .then_with(|| cmp_option_box_term(al, bl))
            .then_with(|| cmp_term(av, bv)),
        (Term::Optional(a), Term::Optional(b))
        | (Term::Pointer(a), Term::Pointer(b))
        | (Term::Lift(a), Term::Lift(b))
        | (Term::ComptimeFunction(a), Term::ComptimeFunction(b))
        | (Term::UnsafeFunction(a), Term::UnsafeFunction(b))
        | (Term::PureFunction(a), Term::PureFunction(b)) => cmp_term(a, b),
        (Term::NormalFormFunction(a, ai), Term::NormalFormFunction(b, bi)) => {
            cmp_term(a, b).then_with(|| ai.cmp(bi))
        }
        (
            Term::Function {
                parameters: ap,
                result: ar,
            },
            Term::Function {
                parameters: bp,
                result: br,
            },
        ) => cmp_slice_by(ap, bp, cmp_parameter).then_with(|| cmp_term(ar, br)),
        (
            Term::Hole {
                index: ai,
                sort: asort,
            },
            Term::Hole {
                index: bi,
                sort: bsort,
            },
        ) => ai.cmp(bi).then_with(|| cmp_sort(*asort, *bsort)),
        (
            Term::Bound {
                depth: ad,
                index: ai,
                sort: asort,
            },
            Term::Bound {
                depth: bd,
                index: bi,
                sort: bsort,
            },
        ) => ad
            .cmp(bd)
            .then_with(|| ai.cmp(bi))
            .then_with(|| cmp_sort(*asort, *bsort)),
        (
            Term::Rigid {
                declaration: ad,
                sort: asort,
            },
            Term::Rigid {
                declaration: bd,
                sort: bsort,
            },
        ) => ad.cmp(bd).then_with(|| cmp_sort(*asort, *bsort)),
        (Term::Runtime(a), Term::Runtime(b)) => a.cmp(b),
        (
            Term::RuntimeField {
                base: ab,
                index: ai,
            },
            Term::RuntimeField {
                base: bb,
                index: bi,
            },
        ) => cmp_term(ab, bb).then_with(|| ai.cmp(bi)),
        (Term::Bool(a), Term::Bool(b)) => a.cmp(b),
        (Term::Char(a), Term::Char(b)) => a.cmp(b),
        (Term::BitVec(a), Term::BitVec(b)) => a
            .width()
            .cmp(&b.width())
            .then_with(|| a.words().cmp(b.words())),
        (Term::Float16(a), Term::Float16(b)) | (Term::BFloat16(a), Term::BFloat16(b)) => a.cmp(b),
        (Term::Float32(a), Term::Float32(b)) => a.cmp(b),
        (Term::Float64(a), Term::Float64(b)) => a.cmp(b),
        (Term::String(a), Term::String(b)) | (Term::Symbol(a), Term::Symbol(b)) => a.cmp(b),
        (Term::MethodValue(a), Term::MethodValue(b)) => a
            .interface
            .declaration
            .cmp(&b.interface.declaration)
            .then_with(|| cmp_term_slice(&a.interface.arguments, &b.interface.arguments))
            .then_with(|| cmp_term(&a.subject, &b.subject))
            .then_with(|| a.member.cmp(&b.member))
            .then_with(|| cmp_term_slice(&a.method_arguments, &b.method_arguments)),
        (
            Term::FunctionValue {
                declaration: ad,
                arguments: aa,
            },
            Term::FunctionValue {
                declaration: bd,
                arguments: ba,
            },
        ) => ad.cmp(bd).then_with(|| cmp_term_slice(aa, ba)),
        (
            Term::ClosureSnapshot {
                factory: af,
                arguments: aa,
            },
            Term::ClosureSnapshot {
                factory: bf,
                arguments: ba,
            },
        ) => af.cmp(bf).then_with(|| cmp_term_slice(aa, ba)),
        (
            Term::ClosureValue {
                ty: at,
                body: ab,
                captures: ac,
            },
            Term::ClosureValue {
                ty: bt,
                body: bb,
                captures: bc,
            },
        ) => cmp_term(at, bt)
            .then_with(|| ab.cmp(bb))
            .then_with(|| cmp_term_slice(ac, bc)),
        (Term::Integer(a), Term::Integer(b)) => a.cmp(b),
        (Term::Float(a), Term::Float(b)) => a
            .is_negative()
            .cmp(&b.is_negative())
            .then_with(|| a.digits().cmp(b.digits()))
            .then_with(|| a.exponent().cmp(b.exponent())),
        (
            Term::Value {
                store: as_,
                index: ai,
            },
            Term::Value {
                store: bs,
                index: bi,
            },
        ) => as_.cmp(bs).then_with(|| ai.cmp(bi)),
        _ => unreachable!("term rank handles mismatched variants"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeclarationId, Primitive};

    #[test]
    fn ordered_term_order_is_total_and_consistent() {
        let a = OrderedTerm(Term::Primitive(Primitive::Bool));
        let b = OrderedTerm(Term::Optional(Box::new(Term::Primitive(Primitive::U32))));
        let c = OrderedTerm(Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            arguments: vec![Term::Primitive(Primitive::Type)],
        });
        let items = [a.clone(), b.clone(), c.clone()];
        for x in &items {
            assert_eq!(x.cmp(x), Ordering::Equal);
            for y in &items {
                assert_eq!(x.cmp(y), y.cmp(x).reverse());
            }
        }
    }
}
