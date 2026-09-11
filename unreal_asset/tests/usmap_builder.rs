//! Builds usmap bytes for tests.
//!
//! The layout is written as literal numbers -- type codes, field widths, the no-super-type sentinel
//! -- rather than derived from the types under test. A fixture built out of `EPropertyType` would
//! re-encode whatever numbering the enum happens to have and agree with itself no matter how wrong
//! it was; these bytes disagree instead.

#![allow(dead_code)]

// usmap header, as found in the wild (verified against a UE4SS-dumped 5.6 mappings file)
pub const MAGIC: u16 = 0x30C4;
pub const VERSION_ENUM_VALUES: u8 = 4;
pub const COMPRESSION_NONE: u8 = 0;

// `EPropertyType` discriminants
pub const T_BYTE: u8 = 0;
pub const T_INT: u8 = 2;
pub const T_NAME: u8 = 5;
pub const T_ARRAY: u8 = 8;
pub const T_STRUCT: u8 = 9;
pub const T_MAP: u8 = 24;
pub const T_SET: u8 = 25;
pub const T_ENUM: u8 = 26;
pub const T_OPTIONAL: u8 = 28;
pub const T_UTF8STR: u8 = 29;
pub const T_ANSISTR: u8 = 30;

/// A property type tree, encoded the way a usmap stores one: a tag byte, then whatever that tag
/// implies -- one nested type for array/set/optional, two for map, a trailing name for enum/struct.
pub enum Type {
    Shallow(u8),
    Array(Box<Type>),
    Set(Box<Type>),
    Optional(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Enum(Box<Type>, String),
    Struct(String),
}

pub fn shallow(code: u8) -> Type {
    Type::Shallow(code)
}

pub struct Prop {
    pub schema_index: u16,
    pub array_size: u8,
    pub name: String,
    pub ty: Type,
}

pub fn prop(schema_index: u16, array_size: u8, name: &str, ty: Type) -> Prop {
    Prop {
        schema_index,
        array_size,
        name: name.to_string(),
        ty,
    }
}

/// Builds usmap bytes. Names are interned as they are referenced, so the name map ends up holding
/// exactly what the enums and schemas pointed at.
#[derive(Default)]
pub struct UsmapBuilder {
    names: Vec<String>,
    enums: Vec<u8>,
    enum_count: u32,
    schemas: Vec<u8>,
    schema_count: u32,
}

impl UsmapBuilder {
    fn name_index(&mut self, name: &str) -> i32 {
        match self.names.iter().position(|e| e == name) {
            Some(index) => index as i32,
            None => {
                self.names.push(name.to_string());
                self.names.len() as i32 - 1
            }
        }
    }

    pub fn add_enum(&mut self, name: &str, entries: &[(i64, &str)]) {
        let name_index = self.name_index(name);

        let mut out = Vec::new();
        out.extend(name_index.to_le_bytes());
        // u16 since LargeEnums
        out.extend((entries.len() as u16).to_le_bytes());
        for (value, entry) in entries {
            // explicit values since EnumValues
            out.extend(value.to_le_bytes());
            let entry_index = self.name_index(entry);
            out.extend(entry_index.to_le_bytes());
        }

        self.enums.extend(out);
        self.enum_count += 1;
    }

    pub fn add_schema(
        &mut self,
        name: &str,
        super_type: Option<&str>,
        prop_count: u16,
        props: &[Prop],
    ) {
        let name_index = self.name_index(name);
        let super_index = match super_type {
            Some(super_type) => self.name_index(super_type),
            // -1 is the "no super type" sentinel
            None => -1,
        };

        let mut out = Vec::new();
        out.extend(name_index.to_le_bytes());
        out.extend(super_index.to_le_bytes());
        out.extend(prop_count.to_le_bytes());
        out.extend((props.len() as u16).to_le_bytes());

        for prop in props {
            out.extend(prop.schema_index.to_le_bytes());
            out.push(prop.array_size);
            let prop_name_index = self.name_index(&prop.name);
            out.extend(prop_name_index.to_le_bytes());
            self.write_type(&mut out, &prop.ty);
        }

        self.schemas.extend(out);
        self.schema_count += 1;
    }

    fn write_type(&mut self, out: &mut Vec<u8>, ty: &Type) {
        match ty {
            Type::Shallow(code) => out.push(*code),
            Type::Array(inner) => {
                out.push(T_ARRAY);
                self.write_type(out, inner);
            }
            Type::Set(inner) => {
                out.push(T_SET);
                self.write_type(out, inner);
            }
            Type::Optional(inner) => {
                out.push(T_OPTIONAL);
                self.write_type(out, inner);
            }
            Type::Map(key, value) => {
                out.push(T_MAP);
                self.write_type(out, key);
                self.write_type(out, value);
            }
            Type::Enum(inner, name) => {
                out.push(T_ENUM);
                self.write_type(out, inner);
                let name_index = self.name_index(name);
                out.extend(name_index.to_le_bytes());
            }
            Type::Struct(name) => {
                out.push(T_STRUCT);
                let name_index = self.name_index(name);
                out.extend(name_index.to_le_bytes());
            }
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut body = Vec::new();

        body.extend((self.names.len() as u32).to_le_bytes());
        for name in &self.names {
            // u16 since LongFName, and the length is the length -- no off-by-one
            body.extend((name.len() as u16).to_le_bytes());
            body.extend(name.as_bytes());
        }

        body.extend(self.enum_count.to_le_bytes());
        body.extend(&self.enums);

        body.extend(self.schema_count.to_le_bytes());
        body.extend(&self.schemas);

        // real files carry a trailing CEXT container of module paths; nothing needs it, and a v4
        // reader must leave it alone rather than parsing it as an extension block
        body.extend(b"CEXT");
        body.extend(0u32.to_le_bytes());

        let mut out = Vec::new();
        out.extend(MAGIC.to_le_bytes());
        out.push(VERSION_ENUM_VALUES);
        // bHasVersioning is four bytes, not one
        out.extend(0i32.to_le_bytes());
        out.push(COMPRESSION_NONE);
        out.extend((body.len() as u32).to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(&body);
        out
    }
}
