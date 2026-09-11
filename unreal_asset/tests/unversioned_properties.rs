//! Guards for reading and writing unversioned properties.
//!
//! An unversioned property carries no tag: its name, type and position all come from the mappings
//! plus the fragment stream in front of it. Nothing in the suite exercised that, because the only
//! archives that admit to having unversioned properties are real assets, and `RawReader` hardcodes
//! `has_unversioned_properties()` to false and `get_mappings()` to `None`.
//!
//! [`MappedReader`] and [`MappedWriter`] are the smallest thing that does admit to it, so the
//! fragment walk, the tagless read/write path and the header generator can all be driven directly
//! off bytes written here.

mod usmap_builder;

use std::io::{Cursor, Read, Seek, Write};

use unreal_asset::{
    containers::{Chain, IndexedMap, NameMap, SharedResource},
    custom_version::{CustomVersion, CustomVersionTrait},
    engine_version::EngineVersion,
    object_version::{ObjectVersion, ObjectVersionUE5},
    properties::{
        array_property::ArrayProperty, generate_unversioned_header, int_property::IntProperty,
        struct_property::StructProperty, Property, PropertyDataTrait,
    },
    reader::{ArchiveReader, ArchiveTrait, ArchiveType, ArchiveWriter, RawReader, RawWriter},
    types::{FName, PackageIndex},
    unversioned::{
        header::{UnversionedHeader, UnversionedHeaderFragment},
        Ancestry, Usmap,
    },
    Error,
};

use usmap_builder::{prop, shallow, Type, UsmapBuilder, T_INT};

/// The schema every test here resolves against: six single-element ints, `P0` through `P5`.
const SCHEMA: &str = "Thing";
const PROP_COUNT: u16 = 6;

/// A 5.6-era pair. These have to be recent enough for property guids to exist at all: below
/// `VER_UE4_PROPERTY_GUID_IN_PROPERTY_TAG` a tagged read skips the guid on its own, and a test
/// asserting that unversioned properties skip it would pass without the code doing anything.
const OBJECT_VERSION: ObjectVersion = ObjectVersion::VER_UE4_PROPERTY_GUID_IN_PROPERTY_TAG;
const OBJECT_VERSION_UE5: ObjectVersionUE5 = ObjectVersionUE5::OS_SUB_OBJECT_SHADOW_SERIALIZATION;

fn mappings() -> Usmap {
    let mut builder = UsmapBuilder::default();
    let props: Vec<_> = (0..PROP_COUNT)
        .map(|index| prop(index, 1, &format!("P{index}"), shallow(T_INT)))
        .collect();
    builder.add_schema(SCHEMA, None, PROP_COUNT, &props);
    Usmap::new(Cursor::new(builder.build())).expect("mappings")
}

fn ancestry() -> Ancestry {
    Ancestry::new(FName::new_dummy(SCHEMA.to_string(), 0))
}

fn int_property(name: &str, value: i32) -> Property {
    IntProperty {
        name: FName::new_dummy(name.to_string(), 0),
        ancestry: ancestry(),
        property_guid: None,
        duplication_index: 0,
        value,
    }
    .into()
}

/// Packs a fragment the way the format does, with the masks written out rather than taken from the
/// private constants that do the unpacking.
fn fragment(skip_num: u8, value_num: u8, has_zeros: bool, is_last: bool) -> u16 {
    skip_num as u16
        | if has_zeros { 0x0080 } else { 0 }
        | ((value_num as u16) << 9)
        | if is_last { 0x0100 } else { 0 }
}

/// A reader that claims unversioned properties and carries mappings; `RawReader` does neither.
struct MappedReader<'mappings> {
    inner: RawReader<PackageIndex, Cursor<Vec<u8>>>,
    mappings: &'mappings Usmap,
}

impl<'mappings> MappedReader<'mappings> {
    fn new(data: Vec<u8>, mappings: &'mappings Usmap) -> Self {
        MappedReader {
            inner: RawReader::new(
                Chain::new(Cursor::new(data), None),
                OBJECT_VERSION,
                OBJECT_VERSION_UE5,
                false,
                NameMap::new(),
            ),
            mappings,
        }
    }
}

impl ArchiveTrait<PackageIndex> for MappedReader<'_> {
    fn get_archive_type(&self) -> ArchiveType {
        self.inner.get_archive_type()
    }

    fn get_custom_version<T>(&self) -> CustomVersion
    where
        T: CustomVersionTrait + Into<i32>,
    {
        self.inner.get_custom_version::<T>()
    }

    fn has_unversioned_properties(&self) -> bool {
        true
    }

    fn use_event_driven_loader(&self) -> bool {
        false
    }

    fn position(&mut self) -> u64 {
        self.inner.position()
    }

    fn get_name_map(&self) -> SharedResource<NameMap> {
        self.inner.get_name_map()
    }

    fn get_array_struct_type_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_array_struct_type_override()
    }

    fn get_map_key_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_map_key_override()
    }

    fn get_map_value_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_map_value_override()
    }

    fn get_engine_version(&self) -> EngineVersion {
        self.inner.get_engine_version()
    }

    fn get_object_version(&self) -> ObjectVersion {
        self.inner.get_object_version()
    }

    fn get_object_version_ue5(&self) -> ObjectVersionUE5 {
        self.inner.get_object_version_ue5()
    }

    fn get_mappings(&self) -> Option<&Usmap> {
        Some(self.mappings)
    }

    fn get_parent_class_export_name(&self) -> Option<FName> {
        None
    }

    fn get_object_name(&self, index: PackageIndex) -> Option<FName> {
        self.inner.get_object_name(index)
    }

    fn get_object_name_packageindex(&self, index: PackageIndex) -> Option<FName> {
        self.inner.get_object_name_packageindex(index)
    }
}

impl ArchiveReader<PackageIndex> for MappedReader<'_> {
    unreal_asset_base::passthrough_archive_reader!(inner);
}

impl Read for MappedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for MappedReader<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// The writing counterpart to [`MappedReader`].
struct MappedWriter<'cursor, 'mappings> {
    inner: RawWriter<'cursor, PackageIndex, Cursor<Vec<u8>>>,
    mappings: &'mappings Usmap,
}

impl<'cursor, 'mappings> MappedWriter<'cursor, 'mappings> {
    fn new(cursor: &'cursor mut Cursor<Vec<u8>>, mappings: &'mappings Usmap) -> Self {
        MappedWriter {
            inner: RawWriter::new(
                cursor,
                OBJECT_VERSION,
                OBJECT_VERSION_UE5,
                false,
                NameMap::new(),
            ),
            mappings,
        }
    }
}

impl ArchiveTrait<PackageIndex> for MappedWriter<'_, '_> {
    fn get_archive_type(&self) -> ArchiveType {
        self.inner.get_archive_type()
    }

    fn get_custom_version<T>(&self) -> CustomVersion
    where
        T: CustomVersionTrait + Into<i32>,
    {
        self.inner.get_custom_version::<T>()
    }

    fn has_unversioned_properties(&self) -> bool {
        true
    }

    fn use_event_driven_loader(&self) -> bool {
        false
    }

    fn position(&mut self) -> u64 {
        self.inner.position()
    }

    fn get_name_map(&self) -> SharedResource<NameMap> {
        self.inner.get_name_map()
    }

    fn get_array_struct_type_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_array_struct_type_override()
    }

    fn get_map_key_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_map_key_override()
    }

    fn get_map_value_override(&self) -> &IndexedMap<String, String> {
        self.inner.get_map_value_override()
    }

    fn get_engine_version(&self) -> EngineVersion {
        self.inner.get_engine_version()
    }

    fn get_object_version(&self) -> ObjectVersion {
        self.inner.get_object_version()
    }

    fn get_object_version_ue5(&self) -> ObjectVersionUE5 {
        self.inner.get_object_version_ue5()
    }

    fn get_mappings(&self) -> Option<&Usmap> {
        Some(self.mappings)
    }

    fn get_parent_class_export_name(&self) -> Option<FName> {
        None
    }

    fn get_object_name(&self, index: PackageIndex) -> Option<FName> {
        self.inner.get_object_name(index)
    }

    fn get_object_name_packageindex(&self, index: PackageIndex) -> Option<FName> {
        self.inner.get_object_name_packageindex(index)
    }
}

impl ArchiveWriter<PackageIndex> for MappedWriter<'_, '_> {
    unreal_asset_base::passthrough_archive_writer!(inner);
}

impl Write for MappedWriter<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for MappedWriter<'_, '_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn an_empty_fragment_has_a_negative_last_num() {
    let empty = UnversionedHeaderFragment {
        skip_num: 0,
        value_num: 0,
        first_num: 0,
        is_last: true,
        has_zeros: false,
    };

    // computed in u8 this wrapped to 255, and every property index compared against it then looked
    // like it was still inside the fragment
    assert_eq!(empty.get_last_num(), -1);
}

#[test]
fn fragment_bits_unpack_to_their_fields() {
    let unpacked = UnversionedHeaderFragment::from(fragment(3, 5, true, true));

    assert_eq!(unpacked.skip_num, 3);
    assert_eq!(unpacked.value_num, 5);
    assert!(unpacked.has_zeros);
    assert!(unpacked.is_last);

    let plain = UnversionedHeaderFragment::from(fragment(0, 1, false, false));
    assert!(!plain.has_zeros);
    assert!(!plain.is_last);
}

/// Two fragments with a gap: one value at index 0, then four skipped, then one at index 5.
fn split_header_bytes(values: &[i32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(fragment(0, 1, false, false).to_le_bytes());
    bytes.extend(fragment(4, 1, false, true).to_le_bytes());
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

#[test]
fn reads_a_split_fragment_header() -> Result<(), Error> {
    let mappings = mappings();
    let mut reader = MappedReader::new(split_header_bytes(&[]), &mappings);

    let header = UnversionedHeader::new(&mut reader)?.expect("header");

    assert_eq!(header.fragments.len(), 2);
    // first_num accumulates skips across fragments, so the second fragment starts past the gap
    assert_eq!(header.fragments[0].first_num, 0);
    assert_eq!(header.fragments[1].first_num, 5);
    assert!(header.fragments[1].is_last);
    assert_eq!(header.unversioned_property_index, 0);
    assert!(header.has_non_zero_values);

    Ok(())
}

#[test]
fn reads_properties_across_a_fragment_gap() -> Result<(), Error> {
    let mappings = mappings();
    let mut reader = MappedReader::new(split_header_bytes(&[111, 555]), &mappings);
    let mut header = UnversionedHeader::new(&mut reader)?.expect("header");

    assert!(
        header.fragments.len() > 1,
        "a single fragment never forces the walk to advance, so nothing here would be tested"
    );

    let mut read = Vec::new();
    while let Some(property) = Property::new(&mut reader, ancestry(), Some(&mut header), true)? {
        read.push(property);
    }

    // the walk has to step over the gap to reach P5; advancing on the wrong side of the comparison
    // runs straight off the end of the fragment list and stops at nothing
    let names: Vec<String> = read
        .iter()
        .map(|property| property.get_name().get_owned_content())
        .collect();
    assert_eq!(names, vec!["P0".to_string(), "P5".to_string()]);

    // an unversioned property is a bare value: reading a property guid ahead of it would consume
    // the value's own bytes and every number after this point would be wrong
    let values: Vec<i32> = read
        .iter()
        .map(|property| match property {
            Property::IntProperty(int) => int.value,
            other => panic!("expected an int, got {other:?}"),
        })
        .collect();
    assert_eq!(values, vec![111, 555]);

    Ok(())
}

#[test]
fn writes_unversioned_properties_without_a_tag() -> Result<(), Error> {
    let mappings = mappings();
    let property = int_property("P0", 111);

    let mut cursor = Cursor::new(Vec::new());
    let written = {
        let mut writer = MappedWriter::new(&mut cursor, &mappings);
        // include_header is true, the way a tagged write would ask for it
        Property::write(&property, &mut writer, true)?
    };

    // just the value: no name, no type, no length, no duplication index, no guid
    assert_eq!(written, 4);
    assert_eq!(cursor.into_inner(), 111i32.to_le_bytes());

    Ok(())
}

#[test]
fn generates_a_header_for_non_contiguous_properties() -> Result<(), Error> {
    let mappings = mappings();
    // positions 0 and 1 in this slice, but schema indices 0 and 5
    let properties = vec![int_property("P0", 111), int_property("P5", 555)];

    let mut cursor = Cursor::new(Vec::new());
    let writer = MappedWriter::new(&mut cursor, &mappings);

    let (header, sorted) = generate_unversioned_header(
        &writer,
        &properties,
        &FName::new_dummy(SCHEMA.to_string(), 0),
    )?
    .expect("header");

    // walking the chunk means counting up through schema indices, which run past the end of the
    // slice they came from -- they have to be mapped back to a position, not used as one
    let names: Vec<String> = sorted
        .iter()
        .map(|property| property.get_name().get_owned_content())
        .collect();
    assert_eq!(names, vec!["P0".to_string(), "P5".to_string()]);

    // a property at schema index 0 is only reachable if "nothing covered yet" means -1; starting
    // the scan at 0 skips straight past it and emits one fragment instead of two
    assert_eq!(header.fragments.len(), 2);
    assert_eq!(header.fragments[0].skip_num, 0);
    assert_eq!(header.fragments[0].value_num, 1);
    assert_eq!(header.fragments[0].first_num, 0);
    assert_eq!(header.fragments[1].skip_num, 4);
    assert_eq!(header.fragments[1].value_num, 1);
    assert!(header.fragments[1].is_last);

    Ok(())
}

#[test]
fn generated_headers_survive_a_round_trip() -> Result<(), Error> {
    let mappings = mappings();
    let properties = vec![int_property("P0", 111), int_property("P5", 555)];

    let mut cursor = Cursor::new(Vec::new());
    let generated = {
        let mut writer = MappedWriter::new(&mut cursor, &mappings);
        let (header, _) = generate_unversioned_header(
            &writer,
            &properties,
            &FName::new_dummy(SCHEMA.to_string(), 0),
        )?
        .expect("header");
        header.write(&mut writer)?;
        header
    };

    let mut reader = MappedReader::new(cursor.into_inner(), &mappings);
    let read = UnversionedHeader::new(&mut reader)?.expect("header");

    assert_eq!(read.fragments, generated.fragments);

    Ok(())
}

/// A holder with one array-of-struct property, and the struct it points at.
fn array_mappings() -> Usmap {
    let mut builder = UsmapBuilder::default();
    builder.add_schema("Inner", None, 1, &[prop(0, 1, "X", shallow(T_INT))]);
    builder.add_schema(
        "Holder",
        None,
        1,
        &[prop(
            0,
            1,
            "Items",
            Type::Array(Box::new(Type::Struct("Inner".to_string()))),
        )],
    );
    Usmap::new(Cursor::new(builder.build())).expect("mappings")
}

fn name(value: &str) -> FName {
    FName::new_dummy(value.to_string(), 0)
}

#[test]
fn round_trips_an_unversioned_array_of_structs() -> Result<(), Error> {
    let mappings = array_mappings();

    let holder = Ancestry::new(name("Holder"));
    let element_ancestry = holder.with_parent(name("Items"));
    let field_ancestry = element_ancestry.with_parent(name("Inner"));

    let element = StructProperty {
        name: name("Items"),
        ancestry: element_ancestry,
        struct_type: Some(name("Inner")),
        struct_guid: None,
        property_guid: None,
        duplication_index: 0,
        serialize_none: true,
        value: vec![IntProperty {
            name: name("X"),
            ancestry: field_ancestry,
            property_guid: None,
            duplication_index: 0,
            value: 7,
        }
        .into()],
    };

    let written: Property = ArrayProperty {
        name: name("Items"),
        ancestry: holder.clone(),
        property_guid: None,
        duplication_index: 0,
        array_type: Some(name("StructProperty")),
        value: vec![element.into()],
        dummy_property: None,
    }
    .into();

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = MappedWriter::new(&mut cursor, &mappings);
        Property::write(&written, &mut writer, true)?;
    }

    let mut reader = MappedReader::new(cursor.into_inner(), &mappings);
    let read = Property::from_type(
        &mut reader,
        &name("ArrayProperty"),
        name("Items"),
        holder,
        // unversioned properties are tagless, so there is no header to include
        false,
        1,
        0,
        0,
        false,
    )?;

    // an inner struct tag on the write side that the read side does not consume leaves every
    // following byte one tag out of step, so the value never survives the trip
    let Property::ArrayProperty(array) = read else {
        panic!("expected an array, got {read:?}");
    };
    assert_eq!(array.value.len(), 1);
    let Property::StructProperty(ref structure) = array.value[0] else {
        panic!("expected a struct, got {:?}", array.value[0]);
    };
    assert_eq!(structure.value.len(), 1);
    let Property::IntProperty(ref int) = structure.value[0] else {
        panic!("expected an int, got {:?}", structure.value[0]);
    };
    assert_eq!(int.value, 7);

    Ok(())
}
