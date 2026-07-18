use std::io::{self, Read, Write as _};
use std::os::unix::net::UnixStream;

use crate::artifact::Artifact;
use crate::build::archive::{self, TailParts};
use crate::build::fanout::FanoutWriter;
use crate::build::router::Router;
use crate::error::Result;
use crate::resolve::BuildPlan;
use crate::source::installer;

struct Routes<'a> {
    stub: Option<UnixStream>,
    data: Option<UnixStream>,
    kernel: Option<&'a mut dyn std::io::Write>,
    cmdline: Option<&'a mut dyn std::io::Write>,
    initramfs: Option<&'a mut dyn std::io::Write>,
}

/// Pulls the installer OCI image once and fans each file to all interested consumers.
pub(crate) async fn pull(
    plan: &BuildPlan,
    router: &mut Router<'_>,
    stub_w: Option<UnixStream>,
    data_w: Option<UnixStream>,
    tail_parts: Option<&TailParts>,
) -> Result<()> {
    let mut route_table = Routes {
        stub: stub_w,
        data: data_w,
        kernel: router.take(Artifact::Kernel),
        cmdline: router.take(Artifact::Cmdline),
        initramfs: router.take(Artifact::Initramfs),
    };

    installer::pull(plan.installer(), &plan.arch(), |path, _size, reader| {
        route(path, reader, &mut route_table, tail_parts)
    })
    .await
}

fn route(
    path: &str,
    reader: &mut dyn Read,
    routes: &mut Routes<'_>,
    tail_parts: Option<&TailParts>,
) -> io::Result<()> {
    match path {
        "stub.efi" => {
            if let Some(w) = routes.stub.as_mut() {
                io::copy(reader, w)?;
            }
        }
        "vmlinuz" => fanout2(reader, &mut routes.data, &mut routes.kernel)?,
        "cmdline" => fanout2(reader, &mut routes.data, &mut routes.cmdline)?,
        "initramfs.img" => {
            fanout_initramfs(reader, &mut routes.data, &mut routes.initramfs, tail_parts)?;
        }
        _ => {}
    }
    Ok(())
}

fn fanout2(
    reader: &mut dyn Read,
    stream: &mut Option<UnixStream>,
    writer_slot: &mut Option<&mut dyn std::io::Write>,
) -> io::Result<()> {
    let mut sinks: Vec<&mut dyn std::io::Write> = Vec::with_capacity(2);
    if let Some(w) = stream.as_mut() {
        sinks.push(w);
    }
    if let Some(w) = writer_slot.as_mut() {
        sinks.push(*w);
    }
    if sinks.is_empty() {
        return Ok(());
    }
    io::copy(reader, &mut FanoutWriter { sinks: &mut sinks })?;
    Ok(())
}

fn fanout_initramfs(
    reader: &mut dyn Read,
    data_stream: &mut Option<UnixStream>,
    initramfs_writer: &mut Option<&mut dyn std::io::Write>,
    tail_parts: Option<&TailParts>,
) -> io::Result<()> {
    let needs_tee = data_stream.is_some() && initramfs_writer.is_some() && tail_parts.is_some();
    if needs_tee {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        if let Some(w) = data_stream.as_mut() {
            w.write_all(&buf)?;
        }
        if let (Some(w), Some(tail)) = (initramfs_writer.as_mut(), tail_parts) {
            w.write_all(&buf)?;
            archive::build_tail_from_parts(tail, w).map_err(io::Error::other)?;
        }
        return Ok(());
    }
    if let Some(w) = data_stream.as_mut() {
        io::copy(reader, w)?;
    }
    if let Some(w) = initramfs_writer.as_mut() {
        io::copy(reader, w)?;
        if let Some(tail) = tail_parts {
            archive::build_tail_from_parts(tail, w).map_err(io::Error::other)?;
        }
    }
    Ok(())
}
