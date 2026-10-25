use super::*;
use nir::program::{FieldOwner, Place};

impl Emitter<'_> {
    pub(super) fn record(
        &mut self,
        source: nir::program::SourceRange,
        fields: &[Expr],
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        if *ty == Term::Primitive(P::Never) {
            for (index, field) in fields.iter().enumerate() {
                let value = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                self.retain_prepared_field(source, index, field.source, &value)?;
            }
            return Err("host Never product construction did not terminate".into());
        }
        let ty = repr(self.graph, ty, self.checked)?;
        if ty == Ty::Unit && fields.is_empty() {
            return Ok(Value::unit());
        }
        let Ty::Record(layout) = &ty else {
            return Err("host anonymous construction lacks a record layout".into());
        };
        if fields.len() != layout.len() {
            return Err("host anonymous field count mismatch".into());
        }
        let mut value = Value {
            ty: ty.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, (field, expected)) in fields.iter().zip(layout.iter()).enumerate() {
            let field_source = field.source;
            let field = self.expr(field, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            if field.ty != *expected {
                return Err("host record field representation mismatch".into());
            }
            self.retain_prepared_field(source, index, field_source, &field)?;
            let (storage, text) = if field.ty == Ty::Unit {
                ("{}".into(), "zeroinitializer".into())
            } else {
                (field.ty.llvm(), field.text)
            };
            value = self.instruction(
                ty.clone(),
                format!(
                    "insertvalue {} {}, {storage} {text}, {index}",
                    ty.llvm(),
                    value.text
                ),
            );
        }
        self.finish_prepared_construction(source)?;
        Ok(value)
    }

    pub(super) fn record_field(&mut self, value: &Value, index: usize) -> Result<Value, String> {
        let Ty::Record(fields) = &value.ty else {
            return Err("host field projection requires an anonymous record".into());
        };
        let field = fields
            .get(index)
            .ok_or("host record projection index out of range")?;
        if *field == Ty::Unit {
            return Ok(Value::unit());
        }
        Ok(self.instruction(
            field.clone(),
            format!("extractvalue {} {}, {index}", value.ty.llvm(), value.text),
        ))
    }

    pub(super) fn assignment_slot(&mut self, place: &Place) -> Result<Slot, String> {
        let root = self
            .locals
            .get(&place.root)
            .cloned()
            .ok_or("host assignment binding is unavailable")?;
        if place.fields.is_empty() {
            return Ok(root);
        }
        let mut ty = root.ty.clone();
        let mut path = String::from("i32 0");
        for (owner, index) in &place.fields {
            if *owner != FieldOwner::Anonymous && !self.checked {
                return Err("host nominal field assignment requires resource verification".into());
            }
            let Ty::Record(fields) = &ty else {
                return Err("host assignment path is not an anonymous record".into());
            };
            ty = fields
                .get(*index)
                .cloned()
                .ok_or("host assignment field index out of range")?;
            path.push_str(&format!(", i32 {index}"));
        }
        let name = self.fresh();
        self.line(format!(
            "%{name} = getelementptr {}, ptr {}, {path}",
            root.ty.llvm(),
            root.pointer
        ));
        Ok(Slot {
            ty,
            pointer: format!("%{name}"),
        })
    }
}
