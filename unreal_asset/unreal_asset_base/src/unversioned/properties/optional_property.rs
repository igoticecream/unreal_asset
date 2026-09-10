//! Optional property

use std::mem::size_of;

use byteorder::WriteBytesExt;

use crate::reader::{ArchiveReader, ArchiveWriter};
use crate::types::PackageIndex;
use crate::unversioned::{usmap_reader::UsmapReader, usmap_writer::UsmapWriter};
use crate::Error;

use super::{EPropertyType, UsmapPropertyData, UsmapPropertyDataTrait};

/// Optional property data
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct UsmapOptionalPropertyData {
    /// Inner optional type
    pub inner_type: Box<UsmapPropertyData>,
}

impl UsmapOptionalPropertyData {
    /// Read a `UsmapOptionalPropertyData` from an asset
    pub fn new<R: ArchiveReader<PackageIndex>>(
        asset: &mut UsmapReader<'_, '_, R>,
    ) -> Result<Self, Error> {
        let inner_type = UsmapPropertyData::new(asset)?;

        Ok(UsmapOptionalPropertyData {
            inner_type: Box::new(inner_type),
        })
    }
}

impl UsmapPropertyDataTrait for UsmapOptionalPropertyData {
    fn write<W: ArchiveWriter<PackageIndex>>(
        &self,
        asset: &mut UsmapWriter<'_, '_, W>,
    ) -> Result<usize, Error> {
        asset.write_u8(EPropertyType::OptionalProperty as u8)?;
        let size = self.inner_type.write(asset)?;
        Ok(size + size_of::<u8>())
    }

    fn get_property_type(&self) -> EPropertyType {
        EPropertyType::OptionalProperty
    }
}
