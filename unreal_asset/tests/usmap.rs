//! Guards for usmap parsing.
//!
//! The mappings loader sat behind a byte-swapped magic check, so it could never run and the bugs
//! behind it had never executed once. These tests pin the wire format with a hand-built fixture;
//! see [`usmap_builder`] for why the bytes are written as literals.

use std::io::Cursor;

use unreal_asset::{
    unversioned::{
        properties::{EPropertyType, UsmapPropertyData, UsmapPropertyDataTrait},
        EUsmapVersion, Usmap,
    },
    Error,
};

mod usmap_builder;

use usmap_builder::{
    prop, shallow, Type, UsmapBuilder, MAGIC, T_ANSISTR, T_BYTE, T_INT, T_NAME, T_UTF8STR,
};

/// A name no `u8` length prefix could describe, so reading lengths as one byte cannot accidentally
/// still work here.
fn long_name() -> String {
    "L".repeat(300)
}

/// One enum, a two-deep schema chain covering every nested type shape, and a schema named by
/// [`long_name`].
fn fixture() -> Vec<u8> {
    let mut builder = UsmapBuilder::default();

    builder.add_enum("EState", &[(0, "Idle"), (1, "Busy"), (7, "Broken")]);

    builder.add_schema(
        "BaseThing",
        None,
        1,
        &[prop(0, 1, "BaseCount", shallow(T_INT))],
    );

    // `Slots` is three elements wide, so it occupies schema indices 1..=3 and everything after it
    // is offset accordingly -- 11 properties from 9 serialized entries
    builder.add_schema(
        "MyStruct",
        Some("BaseThing"),
        11,
        &[
            prop(0, 1, "Count", shallow(T_INT)),
            prop(1, 3, "Slots", Type::Array(Box::new(shallow(T_INT)))),
            prop(4, 1, "Where", Type::Struct("Vector".to_string())),
            prop(5, 1, "Maybe", Type::Optional(Box::new(shallow(T_INT)))),
            prop(6, 1, "Label", shallow(T_UTF8STR)),
            prop(7, 1, "Ansi", shallow(T_ANSISTR)),
            prop(
                8,
                1,
                "State",
                Type::Enum(Box::new(shallow(T_BYTE)), "EState".to_string()),
            ),
            prop(
                9,
                1,
                "Lookup",
                Type::Map(
                    Box::new(shallow(T_INT)),
                    Box::new(Type::Struct("Vector".to_string())),
                ),
            ),
            prop(10, 1, "Tags", Type::Set(Box::new(shallow(T_NAME)))),
        ],
    );

    builder.add_schema(&long_name(), None, 0, &[]);

    builder.build()
}

fn parse() -> Result<Usmap, Error> {
    Usmap::new(Cursor::new(fixture()))
}

#[test]
fn parses_header_and_tables() -> Result<(), Error> {
    let long = long_name();
    assert!(
        long.len() > u8::MAX as usize,
        "fixture proves nothing about name length widths: longest name is {} bytes",
        long.len()
    );

    let usmap = parse()?;

    assert_eq!(usmap.version, EUsmapVersion::EnumValues);
    assert!(
        usmap.name_map.contains(&long),
        "the {}-byte name did not survive the name map",
        long.len()
    );

    // entries keep declaration order; the explicit i64 values are read past, not stored
    assert_eq!(
        usmap.enum_map.get_by_key("EState"),
        Some(&vec![
            "Idle".to_string(),
            "Busy".to_string(),
            "Broken".to_string()
        ])
    );

    assert_eq!(usmap.schemas.len(), 3);

    Ok(())
}

#[test]
fn reads_the_no_super_type_sentinel() -> Result<(), Error> {
    let usmap = parse()?;

    let base = usmap.schemas.get_by_key("BaseThing").expect("BaseThing");
    assert_eq!(base.super_type, "", "-1 should read as no super type");

    let derived = usmap.schemas.get_by_key("MyStruct").expect("MyStruct");
    assert_eq!(derived.super_type, "BaseThing");

    Ok(())
}

#[test]
fn resolves_nested_property_types() -> Result<(), Error> {
    let usmap = parse()?;
    let schema = usmap.schemas.get_by_key("MyStruct").expect("MyStruct");

    let type_of = |name: &str| {
        schema
            .get_property(name, 0)
            .unwrap_or_else(|| panic!("{name} missing"))
            .property_data
            .get_property_type()
    };

    // 28..=30 were missing from the enum entirely, so a mappings file using them failed to parse
    assert_eq!(type_of("Maybe"), EPropertyType::OptionalProperty);
    assert_eq!(type_of("Label"), EPropertyType::Utf8StrProperty);
    assert_eq!(type_of("Ansi"), EPropertyType::AnsiStrProperty);

    assert_eq!(type_of("Count"), EPropertyType::IntProperty);
    assert_eq!(type_of("Slots"), EPropertyType::ArrayProperty);
    assert_eq!(type_of("Where"), EPropertyType::StructProperty);
    assert_eq!(type_of("State"), EPropertyType::EnumProperty);
    assert_eq!(type_of("Lookup"), EPropertyType::MapProperty);
    assert_eq!(type_of("Tags"), EPropertyType::SetProperty);

    // the payload each tag carries has to be consumed in the right order, or every byte after it
    // lands in the wrong field
    let data = |name: &str| &schema.get_property(name, 0).unwrap().property_data;

    match data("Where") {
        UsmapPropertyData::UsmapStructPropertyData(inner) => {
            assert_eq!(inner.struct_type, "Vector")
        }
        other => panic!("Where is {other:?}"),
    }
    match data("State") {
        UsmapPropertyData::UsmapEnumPropertyData(inner) => {
            assert_eq!(inner.name, "EState");
            assert_eq!(
                inner.inner_property.get_property_type(),
                EPropertyType::ByteProperty
            );
        }
        other => panic!("State is {other:?}"),
    }
    match data("Lookup") {
        UsmapPropertyData::UsmapMapPropertyData(inner) => {
            assert_eq!(
                inner.inner_type.get_property_type(),
                EPropertyType::IntProperty
            );
            assert_eq!(
                inner.value_type.get_property_type(),
                EPropertyType::StructProperty
            );
        }
        other => panic!("Lookup is {other:?}"),
    }
    match data("Slots") {
        UsmapPropertyData::UsmapArrayPropertyData(inner) => assert_eq!(
            inner.inner_type.get_property_type(),
            EPropertyType::IntProperty
        ),
        other => panic!("Slots is {other:?}"),
    }
    match data("Maybe") {
        UsmapPropertyData::UsmapOptionalPropertyData(inner) => assert_eq!(
            inner.inner_type.get_property_type(),
            EPropertyType::IntProperty
        ),
        other => panic!("Maybe is {other:?}"),
    }

    Ok(())
}

#[test]
fn array_size_expands_into_duplication_indices() -> Result<(), Error> {
    let usmap = parse()?;
    let schema = usmap.schemas.get_by_key("MyStruct").expect("MyStruct");

    // a one-wide property expands to a single entry at index 0 and proves nothing: lookups are by
    // duplication index but the entries were once keyed by schema index, which only differ here
    let slots = schema.get_property("Slots", 0).expect("Slots");
    assert!(
        slots.array_size > 1,
        "fixture has no static array, so schema and duplication indices cannot disagree"
    );

    for duplication_index in 0..slots.array_size as u32 {
        let element = schema
            .get_property("Slots", duplication_index)
            .unwrap_or_else(|| panic!("Slots[{duplication_index}] missing"));

        assert_eq!(element.array_index as u32, duplication_index);
        // the element's slot in the schema, offset from where the property started
        assert_eq!(element.schema_index as u32, 1 + duplication_index);
    }

    assert!(
        schema
            .get_property("Slots", slots.array_size as u32)
            .is_none(),
        "expanded past the array size"
    );

    Ok(())
}

#[test]
fn walks_the_super_type_chain() -> Result<(), Error> {
    let usmap = parse()?;

    let names: Vec<&str> = usmap
        .get_all_properties("MyStruct")
        .iter()
        .map(|property| property.name.as_str())
        .collect();

    // `Slots` contributes one entry per element
    assert_eq!(names.iter().filter(|name| **name == "Slots").count(), 3);
    assert!(
        names.contains(&"BaseCount"),
        "stopped before reaching BaseThing: {names:?}"
    );

    Ok(())
}

#[test]
fn rejects_a_byte_swapped_magic() {
    let mut bytes = fixture();
    bytes.swap(0, 1);

    assert!(
        Usmap::new(Cursor::new(bytes)).is_err(),
        "0x{MAGIC:04X} was accepted with its bytes the wrong way round"
    );
}
