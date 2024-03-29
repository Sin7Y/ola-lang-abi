use serde::{Deserialize, Serialize};
use std::{collections::HashMap, rc::Rc};

use crate::{types::Type, Value};

/// ABI decoded param value.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DecodedParam {
    // Param definition.
    pub param: Param,
    // Decoded param value.
    pub value: Value,
}

impl From<(Param, Value)> for DecodedParam {
    fn from((param, value): (Param, Value)) -> Self {
        Self { param, value }
    }
}

/// ABI decoded values. Fast access by param index and name.
///
/// This struct provides a way for accessing decoded param values by index and by name.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DecodedParams(Vec<DecodedParam>);

impl DecodedParams {
    /// Creates a reader.
    ///
    /// Parameters are indexed by name at reader creation.
    pub fn reader(&self) -> DecodedParamsReader {
        DecodedParamsReader::new(self)
    }
}

impl std::ops::Deref for DecodedParams {
    type Target = Vec<DecodedParam>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<(Param, Value)>> for DecodedParams {
    fn from(values: Vec<(Param, Value)>) -> Self {
        Self(values.into_iter().map(From::from).collect())
    }
}

/// Provides fast read access to decoded params by parameter index and name.
pub struct DecodedParamsReader<'a> {
    /// Decoded params by parameter index.
    pub by_index: Vec<&'a DecodedParam>,
    /// Decoded params by parameter name.
    pub by_name: HashMap<&'a str, &'a DecodedParam>,
}

impl<'a> DecodedParamsReader<'a> {
    fn new(decoded_params: &'a DecodedParams) -> Self {
        let by_index = decoded_params.iter().collect();

        let by_name = decoded_params
            .iter()
            .filter(|decoded_param| !decoded_param.param.name.is_empty())
            .map(|decoded_param| (decoded_param.param.name.as_str(), decoded_param))
            .collect();

        DecodedParamsReader { by_index, by_name }
    }
}

/// A definition of a parameter of a function or event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub type_: Type,
    /// Whether it is an indexed parameter (events only).
    pub indexed: Option<bool>,
}

impl Param {
    fn build_param_entry(&self) -> ParamEntry {
        let tuple_params = match &self.type_ {
            Type::Tuple(params) => Some(params.clone()),
            Type::Array(ty) | Type::FixedArray(ty, _) => {
                if let Type::Tuple(params) = ty.as_ref() {
                    Some(params.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        let components = tuple_params.map(|params| {
            params
                .iter()
                .map(|(name, ty)| {
                    Param {
                        name: name.clone(),
                        type_: ty.clone(),
                        indexed: None,
                    }
                    .build_param_entry()
                })
                .collect()
        });

        ParamEntry {
            name: self.name.clone(),
            type_: param_type_string(&self.type_),
            indexed: self.indexed,
            components,
        }
    }
}

impl Serialize for Param {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.build_param_entry().serialize(serializer)
    }
}

impl<'a> Deserialize<'a> for Param {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let entry: ParamEntry = Deserialize::deserialize(deserializer)?;

        let (_, ty) = parse_exact_type(Rc::new(entry.components), &entry.type_)
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;

        Ok(Param {
            name: entry.name.to_string(),
            type_: ty,
            indexed: entry.indexed,
        })
    }
}

fn param_type_string(ty: &Type) -> String {
    match ty {
        Type::Tuple(_) => String::from("tuple"),
        Type::Array(ty) => format!("{}[]", param_type_string(ty)),
        Type::FixedArray(ty, size) => format!("{}[{}]", param_type_string(ty), size),
        _ => format!("{}", ty),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParamEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ParamEntry>>,
}

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, digit1},
    combinator::{all_consuming, map_res, opt, recognize},
    multi::many1,
    sequence::delimited,
    IResult,
};

#[derive(Debug)]
enum TypeParseError<I> {
    Error,
    NomError(nom::error::Error<I>),
}

impl<I> From<nom::error::Error<I>> for TypeParseError<I> {
    fn from(err: nom::error::Error<I>) -> Self {
        Self::NomError(err)
    }
}

impl<I> nom::error::ParseError<I> for TypeParseError<I> {
    fn from_error_kind(input: I, kind: nom::error::ErrorKind) -> Self {
        Self::NomError(nom::error::Error::new(input, kind))
    }

    fn append(_: I, _: nom::error::ErrorKind, other: Self) -> Self {
        other
    }
}

type TypeParseResult<I, O> = IResult<I, O, TypeParseError<I>>;

fn map_error<I, O>(res: IResult<I, O>) -> TypeParseResult<I, O> {
    res.map_err(|err| err.map(From::from))
}

fn parse_exact_type(
    components: Rc<Option<Vec<ParamEntry>>>,
    input: &str,
) -> TypeParseResult<&str, Type> {
    all_consuming(parse_type(components))(input)
}

fn parse_type(
    components: Rc<Option<Vec<ParamEntry>>>,
) -> impl Fn(&str) -> TypeParseResult<&str, Type> {
    move |input: &str| {
        alt((
            parse_array(components.clone()),
            parse_simple_type(components.clone()),
        ))(input)
    }
}

fn parse_simple_type(
    components: Rc<Option<Vec<ParamEntry>>>,
) -> impl Fn(&str) -> TypeParseResult<&str, Type> {
    move |input: &str| {
        alt((
            parse_tuple(components.clone()),
            parse_fields,
            parse_u32,
            parse_u256,
            parse_field,
            parse_address,
            parse_hash,
            parse_bool,
            parse_string,
        ))(input)
    }
}

fn parse_u32(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("u32")(input).map(|(i, _)| (i, Type::U32)))
}

fn parse_u256(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("u256")(input).map(|(i, _)| (i, Type::U256)))
}

fn parse_field(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("field")(input).map(|(i, _)| (i, Type::Field)))
}

fn parse_address(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("address")(input).map(|(i, _)| (i, Type::Address)))
}

fn parse_hash(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("hash")(input).map(|(i, _)| (i, Type::Hash)))
}

fn parse_bool(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("bool")(input).map(|(i, _)| (i, Type::Bool)))
}

fn parse_string(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("string")(input).map(|(i, _)| (i, Type::String)))
}

fn parse_fields(input: &str) -> TypeParseResult<&str, Type> {
    map_error(tag("fields")(input).map(|(i, _)| (i, Type::Fields)))
}

fn parse_array(
    components: Rc<Option<Vec<ParamEntry>>>,
) -> impl Fn(&str) -> TypeParseResult<&str, Type> {
    move |input: &str| {
        let (i, ty) = parse_simple_type(components.clone())(input)?;

        let (i, sizes) = map_error(many1(delimited(char('['), opt(parse_integer), char(']')))(
            i,
        ))?;

        let array_from_size = |ty: Type, size: Option<u64>| match size {
            None => Type::Array(Box::new(ty)),
            Some(size) => Type::FixedArray(Box::new(ty), size),
        };

        let init_arr_ty = array_from_size(ty, sizes[0]);
        let arr_ty = sizes.into_iter().skip(1).fold(init_arr_ty, array_from_size);

        Ok((i, arr_ty))
    }
}

fn parse_tuple(
    components: Rc<Option<Vec<ParamEntry>>>,
) -> impl Fn(&str) -> TypeParseResult<&str, Type> {
    move |input: &str| {
        let (i, _) = map_error(tag("tuple")(input))?;

        let tys = match components.clone().as_ref() {
            Some(cs) => cs
                .clone()
                .into_iter()
                .try_fold(vec![], |mut param_tys, param| {
                    let comps = param.components.as_ref().cloned();

                    let ty = match parse_exact_type(Rc::new(comps), &param.type_) {
                        Ok((_, ty)) => ty,
                        Err(_) => return Err(nom::Err::Failure(TypeParseError::Error)),
                    };

                    param_tys.push((param.name, ty));

                    Ok(param_tys)
                }),

            None => Err(nom::Err::Failure(TypeParseError::Error)),
        }?;

        Ok((i, Type::Tuple(tys)))
    }
}

fn parse_integer(input: &str) -> IResult<&str, u64> {
    map_res(recognize(many1(digit1)), str::parse)(input)
}

#[cfg(test)]
mod test {
    use super::*;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn serde_u32() {
        let v = json!({
            "name": "a",
            "type": "u32",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::U32,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_u256() {
        let v = json!({
            "name": "a",
            "type": "u256",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::U256,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_field() {
        let v = json!({
            "name": "a",
            "type": "field",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Field,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_address() {
        let v = json!({
            "name": "a",
            "type": "address",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Address,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_bool() {
        let v = json!({
            "name": "a",
            "type": "bool",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Bool,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_string() {
        let v = json!({
            "name": "a",
            "type": "string",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::String,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_fields() {
        let v = json!({
            "name": "a",
            "type": "fields",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Fields,
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_array() {
        let v = json!({
            "name": "a",
            "type": "u32[]",
        });
        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Array(Box::new(Type::U32)),
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_nested_array() {
        let v = json!({
            "name": "a",
            "type": "address[][]",
        });
        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Array(Box::new(Type::Array(Box::new(Type::Address)))),
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_mixed_array() {
        let v = json!({
            "name": "a",
            "type": "string[2][]",
        });
        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::Array(Box::new(Type::FixedArray(Box::new(Type::String), 2))),
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);

        let v = json!({
            "name": "a",
            "type": "string[][3]",
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "a".to_string(),
                type_: Type::FixedArray(Box::new(Type::Array(Box::new(Type::String))), 3),
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }

    #[test]
    fn serde_tuple() {
        let v = json!({
          "name": "s",
          "type": "tuple",
          "components": [
            {
              "name": "a",
              "type": "u32"
            },
            {
              "name": "b",
              "type": "u32[]"
            },
            {
              "name": "c",
              "type": "tuple[]",
              "components": [
                {
                  "name": "x",
                  "type": "u32"
                },
                {
                  "name": "y",
                  "type": "u32"
                }
              ]
            }
          ]
        });

        let param: Param = serde_json::from_value(v.clone()).expect("param deserialized");

        assert_eq!(
            param,
            Param {
                name: "s".to_string(),
                type_: Type::Tuple(vec![
                    ("a".to_string(), Type::U32),
                    ("b".to_string(), Type::Array(Box::new(Type::U32))),
                    (
                        "c".to_string(),
                        Type::Array(Box::new(Type::Tuple(vec![
                            ("x".to_string(), Type::U32),
                            ("y".to_string(), Type::U32)
                        ])))
                    )
                ]),
                indexed: None
            }
        );

        let param_json = serde_json::to_value(param).expect("param serialized");

        assert_eq!(v, param_json);
    }
}
