//! List operations preserve their complete domain, independently of element sort.
use crate::type_links::TypeLinks;
use nir::program::{Expr, ExprKind as K};
use nir::{Primitive, Term};

fn element<'a>(links: &TypeLinks<'_>, ty: &'a Term, fuel: &mut usize) -> Result<&'a Term, String> {
    links.list_element(ty, fuel)?.ok_or_else(|| {
        "IIR list construction or operation requires registered list layout metadata".into()
    })
}
fn equal(left: &Term, right: &Term, fuel: &mut usize) -> Result<bool, String> {
    nir::type_computations::charge_term(left, fuel)
        .map_err(|_| "IIR list domain budget exhausted")?;
    nir::type_computations::charge_term(right, fuel)
        .map_err(|_| "IIR list domain budget exhausted")?;
    Ok(left == right)
}

pub(super) fn validate(links: &TypeLinks<'_>, expr: &Expr, fuel: &mut usize) -> Result<(), String> {
    let never = Term::Primitive(Primitive::Never);
    let integer = Term::Primitive(Primitive::Integer);
    match &expr.kind {
        K::TypeList(fields) => {
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or("IIR list budget exhausted")?;
            if fields.iter().any(|field| field.ty == never) {
                if expr.ty != never {
                    return Err("IIR diverging list construction must be Never".into());
                }
                return Ok(());
            }
            let element = element(links, &expr.ty, fuel)?;
            for field in fields {
                if !equal(&field.ty, element, fuel)? {
                    return Err("IIR list operand does not match its element type".into());
                }
            }
        }
        K::TypeListIndex(values, index) => {
            let element = element(links, &values.ty, fuel)?;
            let expected = if index.ty == never { &never } else { element };
            if (index.ty != integer && index.ty != never) || !equal(&expr.ty, expected, fuel)? {
                return Err("IIR list index has incompatible index or result types".into());
            }
        }
        K::ListLength(values) => {
            let expected = if values.ty == never {
                &never
            } else {
                element(links, &values.ty, fuel)?;
                &integer
            };
            if !equal(&expr.ty, expected, fuel)? {
                return Err("IIR list length has an incompatible result type".into());
            }
        }
        K::ListConcat(left, right) => {
            if left.ty != never {
                element(links, &left.ty, fuel)?;
            }
            if right.ty != never {
                element(links, &right.ty, fuel)?;
            }
            let expected = if left.ty == never || right.ty == never {
                &never
            } else {
                if !equal(&left.ty, &right.ty, fuel)? {
                    return Err("IIR list concatenation requires identical list domains".into());
                }
                &left.ty
            };
            if !equal(&expr.ty, expected, fuel)? {
                return Err("IIR list concatenation has an incompatible result type".into());
            }
        }
        _ => {}
    }
    Ok(())
}
