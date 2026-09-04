use crate::events::Event;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub struct EventLog {
    writer: BufWriter<std::fs::File>,
}

impl EventLog {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    pub fn append(&mut self, event: &Event) {
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(self.writer, "{}", line);
        }
    }

    pub fn append_many(&mut self, events: &[Event]) {
        for e in events {
            self.append(e);
        }
    }

    pub fn flush(&mut self) {
        let _ = self.writer.flush();
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        self.flush();
    }
}
