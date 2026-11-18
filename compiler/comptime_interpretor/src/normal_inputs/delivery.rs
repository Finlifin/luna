//! A known domain constructs ordinary values field by field. Pending fields
//! retain their source positions until the original invocation can admit them.
use super::*;

pub(super) trait Delivery {
    type Item: From<Value>;
    type List;

    fn literal(&mut self, key: InputKey, source: SourceRange) -> Result<Self::Item, Error>;
    fn list(&mut self, key: &InputKey, source: SourceRange) -> Result<Self::List, Error>;
    fn append(
        list: &mut Self::List,
        vm: &mut Interpreter<'_>,
        item: Self::Item,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error>;
    fn finish_list(list: Self::List, vm: &mut Interpreter<'_>) -> Result<Self::Item, Error>;
    fn tuple(key: InputKey, items: Vec<Self::Item>) -> Self::Item;
}

pub(super) struct Ready<'a>(pub(super) &'a mut BTreeMap<InputKey, PreparedNormalInput>);

impl Delivery for Ready<'_> {
    type Item = Value;
    type List = (Term, Vec<Value>);

    fn literal(&mut self, key: InputKey, source: SourceRange) -> Result<Value, Error> {
        match self.0.remove(&key) {
            Some(PreparedNormalInput::Value(value)) => Ok(value),
            _ => Err(Error {
                kind: ErrorKind::Type,
                source: Some(source),
            }),
        }
    }
    fn list(&mut self, key: &InputKey, source: SourceRange) -> Result<Self::List, Error> {
        match self.0.remove(key) {
            Some(PreparedNormalInput::ListType(ty)) => Ok((ty, Vec::new())),
            _ => Err(Error {
                kind: ErrorKind::Type,
                source: Some(source),
            }),
        }
    }
    fn append(
        (ty, values): &mut Self::List,
        vm: &mut Interpreter<'_>,
        item: Value,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        vm.append_normal_list_item(ty, values, spread, item, source)
    }
    fn finish_list((ty, values): Self::List, vm: &mut Interpreter<'_>) -> Result<Value, Error> {
        vm.finish_normal_list(ty, values)
    }
    fn tuple(_: InputKey, items: Vec<Value>) -> Value {
        Value::Tuple(items)
    }
}

pub(super) struct Staged;

impl From<Value> for StagedNormalInput {
    fn from(value: Value) -> Self {
        Self::Value(value)
    }
}

impl Delivery for Staged {
    type Item = StagedNormalInput;
    type List = (InputKey, Vec<ListField>);

    fn literal(&mut self, key: InputKey, _: SourceRange) -> Result<Self::Item, Error> {
        Ok(StagedNormalInput::Literal(key))
    }
    fn list(&mut self, key: &InputKey, _: SourceRange) -> Result<Self::List, Error> {
        Ok((key.clone(), Vec::new()))
    }
    fn append(
        (_, items): &mut Self::List,
        _: &mut Interpreter<'_>,
        item: Self::Item,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        items.push(ListField {
            spread,
            source,
            value: item,
        });
        Ok(())
    }
    fn finish_list((key, items): Self::List, _: &mut Interpreter<'_>) -> Result<Self::Item, Error> {
        Ok(StagedNormalInput::List { key, items })
    }
    fn tuple(key: InputKey, items: Vec<Self::Item>) -> Self::Item {
        StagedNormalInput::Tuple { key, items }
    }
}
