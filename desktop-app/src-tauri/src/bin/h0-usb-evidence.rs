#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, path::PathBuf};

    use hidapi::HidApi;
    use serde_json::json;

    const VID: u16 = 0xcafe;
    const PID: u16 = 0xab01;
    const USAGE_PAGE: u16 = 0xffab;
    const USAGE: u16 = 0x0001;
    const EXPECTED_REPORT_DESCRIPTOR: &[u8] =
        include_bytes!("../../../../firmware/descriptors/h0-report-descriptor.bin");

    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: h0-usb-evidence OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    let descriptor_path = output.join("actual-h0-report-descriptor.bin");
    let record_path = output.join("actual-h0-hidapi-enumeration.json");
    if descriptor_path.exists() || record_path.exists() {
        return Err("refusing to overwrite existing physical evidence".into());
    }

    let api = HidApi::new()?;
    let info = api
        .device_list()
        .find(|info| {
            info.vendor_id() == VID
                && info.product_id() == PID
                && info.usage_page() == USAGE_PAGE
                && info.usage() == USAGE
        })
        .ok_or("the exact H0 Vendor HID collection is not enumerated")?;
    let interface_number = info.interface_number();
    let device = info.open_device(&api)?;
    let mut descriptor = [0u8; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let descriptor_length = device.get_report_descriptor(&mut descriptor)?;
    let descriptor = &descriptor[..descriptor_length];
    if descriptor != EXPECTED_REPORT_DESCRIPTOR {
        return Err("physical HID report descriptor differs from the frozen H0 descriptor".into());
    }

    fs::write(&descriptor_path, descriptor)?;
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&json!({
            "source": "physical hidapi enumeration",
            "vendorId": VID,
            "productId": PID,
            "usagePage": USAGE_PAGE,
            "usage": USAGE,
            "interfaceNumber": interface_number,
            "reportDescriptorBytes": descriptor_length,
            "matchesFrozenDescriptor": true,
            "serialNumberRecorded": false
        }))?,
    )?;
    println!("{}", descriptor_path.display());
    println!("{}", record_path.display());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("H0 USB evidence capture is supported only on macOS.");
    std::process::exit(2);
}
