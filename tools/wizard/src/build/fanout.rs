use std::io::{self, Write};

pub(crate) struct FanoutWriter<'a> {
    pub sinks: &'a mut [&'a mut (dyn Write + Send)],
}

impl Write for FanoutWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for sink in &mut *self.sinks {
            sink.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        for sink in &mut *self.sinks {
            sink.flush()?;
        }
        Ok(())
    }
}
