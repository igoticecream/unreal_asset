use std::io::Cursor;

use unreal_asset::{engine_version::EngineVersion, exports::ExportBaseTrait, Asset, Error};

macro_rules! assets_folder {
    () => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/assets/ue5_6/")
    };
}

const ASSET: &[u8] = include_bytes!(concat!(
    assets_folder!(),
    "BP_MapGimmick_TreasureBox_Hook.uasset"
));
const BULK: &[u8] = include_bytes!(concat!(
    assets_folder!(),
    "BP_MapGimmick_TreasureBox_Hook.uexp"
));

/// A name merged in by [`Asset::rebuild_name_map`] is appended to the end of the name map, so it
/// lands past `names_referenced_from_export_data_count` unless that boundary is widened. Consumers
/// that trust the boundary then drop the name while export data still refers to it -- the
/// zen/iostore name map is truncated to exactly this count, which shows up at load time as
/// `Bad name index <index>/<count>`.
#[test]
fn names_referenced_covers_merged_names() -> Result<(), Error> {
    let mut asset = Asset::new(
        Cursor::new(ASSET),
        Some(Cursor::new(BULK)),
        EngineVersion::VER_UE5_6,
        None,
    )?;

    let before = asset
        .get_name_map()
        .get_ref()
        .get_name_map_index_list()
        .len();

    // without a gap between the boundary and the end of the map there is nothing to regress
    assert!(
        asset.names_referenced_from_export_data_count < before as i32,
        "fixture exposes nothing: {before} names with a boundary of {}",
        asset.names_referenced_from_export_data_count
    );

    // name an export through a foreign name map, the way transplanting between two assets does,
    // so the rebuild has to merge the name in rather than reuse an existing index
    let mut foreign = asset.get_name_map().clone_resource();
    let merged = foreign.get_mut().add_fname("RebuildNameMapProbe");
    asset.asset_data.exports[0]
        .get_base_export_mut()
        .object_name = merged;

    asset.rebuild_name_map();

    let after = asset
        .get_name_map()
        .get_ref()
        .get_name_map_index_list()
        .len();
    assert!(
        after > before,
        "the rebuild merged nothing, so the boundary was never exercised"
    );
    assert!(
        asset.names_referenced_from_export_data_count >= after as i32,
        "name at index {} is referenced from export data but the boundary is {}",
        after - 1,
        asset.names_referenced_from_export_data_count
    );

    Ok(())
}
