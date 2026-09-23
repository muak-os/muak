//! Async `/dev/kmsg` reader that replays the kernel ring buffer and streams new entries.

use std::fs::File;
use std::io::{self, BufRead as _, BufReader, Seek as _, SeekFrom};

use anyhow::{Context as _, Result};
use rustix::fs::{Mode, OFlags, open};
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

const KMSG_PATH: &str = "/dev/kmsg";

const KMSG_CHANNEL_LINES: usize = 256;

const READ_BUFFER: usize = 16 * 1024;

/// Spawns a task that reads all historical and future kernel log entries.
pub fn spawn() -> Result<mpsc::Receiver<String>> {
    let (tx, rx) = mpsc::channel(KMSG_CHANNEL_LINES);

    let fd = open(
        KMSG_PATH,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .context("failed to open /dev/kmsg")?;

    let file = File::from(fd);

    tokio::spawn(async move {
        let _run_result = run(file, tx).await;
    });

    Ok(rx)
}

async fn run(file: File, tx: mpsc::Sender<String>) -> Result<()> {
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;

    {
        let mut reader = BufReader::with_capacity(READ_BUFFER, &file);
        drain(&mut reader, &tx).await?;
    }

    let async_fd = AsyncFd::new(file)?;
    let mut reader = BufReader::with_capacity(READ_BUFFER, async_fd.get_ref());

    loop {
        let mut guard = async_fd.readable().await?;
        guard.clear_ready();

        drain(&mut reader, &tx).await?;
    }
}

async fn drain(reader: &mut BufReader<&File>, tx: &mpsc::Sender<String>) -> Result<()> {
    let mut line_buf = String::new();

    loop {
        line_buf.clear();
        match reader.read_line(&mut line_buf) {
            Ok(0) => return Ok(()),
            Ok(_) => send_entry(&line_buf, tx).await?,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

async fn send_entry(line: &str, tx: &mpsc::Sender<String>) -> Result<()> {
    let Some(text) = parse_entry(line) else {
        return Ok(());
    };

    tx.send(text).await?;

    Ok(())
}

fn parse_entry(line: &str) -> Option<String> {
    let (prefix, text) = line.split_once(';')?;
    let text = text.trim_end_matches('\n');

    let timestamp = prefix
        .split(',')
        .nth(2)
        .and_then(|part| part.parse::<u64>().ok());

    Some(match timestamp {
        Some(usec) => {
            let secs = usec.div_euclid(1_000_000);
            let frac = usec.rem_euclid(1_000_000);
            format!("[{secs:5}.{frac:06}] {text}")
        }
        None => text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entry_formats_timestamp_and_text() {
        // ARRANGE
        let line = "6,1234,3795113,-;kernel booted successfully";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(
            result,
            Some("[    3.795113] kernel booted successfully".to_owned())
        );
    }

    #[test]
    fn parse_entry_trims_trailing_whitespace_on_text() {
        // ARRANGE
        let line = "6,1,2000000,-;  hello world  ";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(result, Some("[    2.000000]   hello world  ".to_owned()));
    }

    #[test]
    fn parse_entry_preserves_leading_spaces() {
        // ARRANGE
        let line = "6,1,1000000,-;        :::   :::";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(result, Some("[    1.000000]         :::   :::".to_owned()));
    }

    #[test]
    fn parse_entry_empty_text_returns_blank_line() {
        // ARRANGE
        let line = "6,1,2,-;";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(result, Some("[    0.000002] ".to_owned()));
    }

    #[test]
    fn parse_entry_no_semicolon_returns_none() {
        // ARRANGE
        let line = "6,1,2,-";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn parse_entry_text_with_semicolons_preserved() {
        // ARRANGE
        let line = "6,1,1000000,-;key=val; extra";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(result, Some("[    1.000000] key=val; extra".to_owned()));
    }

    #[test]
    fn parse_entry_zero_timestamp() {
        // ARRANGE
        let line = "6,1,0,-;early boot";

        // ACT
        let result = parse_entry(line);

        // ASSERT
        assert_eq!(result, Some("[    0.000000] early boot".to_owned()));
    }
}
