use flate2::read::GzDecoder;
use oci_client::client::ImageData;
use oci_client::{Client, Reference, secrets::RegistryAuth};
use std::error::Error;
use std::path::PathBuf;
use tar::Archive;

pub fn pull_and_extract(image: &str) -> Result<PathBuf, Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().to_path_buf();

    let image_ref: Reference = image.parse()?;
    let client = Client::new(Default::default());
    let auth = RegistryAuth::Anonymous;

    let runtime = tokio::runtime::Runtime::new()?;
    let image_data = runtime.block_on(client.pull(&image_ref, &auth, vec![]))?;

    extract_oci_layers(&image_data, &temp_path)?;

    Ok(temp.keep())
}

fn extract_oci_layers(
    image_data: &ImageData,
    dest: &std::path::Path,
) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(dest)?;

    for layer in &image_data.layers {
        extract_layer(&layer.data, dest)?;
    }

    Ok(())
}

fn extract_layer(layer_data: &[u8], dest: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let decoder = GzDecoder::new(layer_data);
    let mut archive = Archive::new(decoder);
    archive.unpack(dest)?;
    Ok(())
}
