use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{GetVmSerialLogRequest, VmServiceClient};

/// Gets VM serial logs.
pub async fn handle(client: &mut VmServiceClient<Channel>, vm_id: String, tail: i64) -> Result<()> {
    let request = tonic::Request::new(GetVmSerialLogRequest {
        vm_id: vm_id.clone(),
        tail_lines: tail,
    });

    let response = client.get_vm_serial_log(request).await?;
    let resp = response.into_inner();

    if resp.error.is_empty() {
        print!("{}", resp.output);
    } else {
        eprintln!(
            "{}",
            format!("Error getting VM serial log: {}", resp.error).red()
        );
        std::process::exit(1);
    }

    Ok(())
}
