//! Streaming writer for JAC files

use jac_codec::{BlockBuilder, BlockData, CompressOpts};
use jac_format::{BlockIndexEntry, FileHeader, IndexFooter, JacError, Result};
use std::io::Write;

/// JAC writer for streaming compression
pub struct JacWriter<W: Write> {
    writer: Option<W>,
    opts: CompressOpts,
    block_builder: BlockBuilder,
    block_index: Vec<BlockIndexEntry>,
    current_offset: u64,
    finished: bool,
    metrics: WriterMetrics,
}

impl<W: Write> JacWriter<W> {
    /// Create new writer
    pub fn new(mut writer: W, header: FileHeader, opts: CompressOpts) -> Result<Self> {
        // Write file header
        let header_bytes = header.encode()?;
        writer.write_all(&header_bytes)?;

        let block_builder = BlockBuilder::new(opts.clone());

        let metrics = WriterMetrics {
            bytes_written: header_bytes.len() as u64,
            ..WriterMetrics::default()
        };

        Ok(Self {
            writer: Some(writer),
            opts,
            block_builder,
            block_index: Vec::new(),
            current_offset: header_bytes.len() as u64,
            finished: false,
            metrics,
        })
    }

    /// Write record to current block
    pub fn write_record(&mut self, rec: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
        // If block is full, flush it first
        if self.block_builder.is_full() {
            self.flush_block()?;
        }

        self.block_builder.add_record(rec.clone())?;
        self.metrics.records_written += 1;
        Ok(())
    }

    /// Write multiple records from an iterator.
    pub fn write_records<I>(&mut self, records: I) -> Result<()>
    where
        I: IntoIterator<Item = serde_json::Map<String, serde_json::Value>>,
    {
        for record in records {
            self.write_record(&record)?;
        }
        Ok(())
    }

    /// Flush current block to output
    fn flush_block(&mut self) -> Result<()> {
        if self.block_builder.record_count() == 0 {
            return Ok(());
        }

        // Finalize the block (this consumes the block builder)
        let block_data = std::mem::replace(
            &mut self.block_builder,
            BlockBuilder::new(self.opts.clone()),
        )
        .finalize()?;
        let block_bytes = self.encode_block(&block_data)?;

        // Track block index entry
        let block_offset = self.current_offset;
        let block_size = block_bytes.len();
        let record_count = block_data.header.record_count;

        self.block_index.push(BlockIndexEntry {
            block_offset,
            block_size,
            record_count,
        });
        self.metrics.blocks_written += 1;

        // Write block to output
        if let Some(writer) = self.writer.as_mut() {
            writer.write_all(&block_bytes)?;
        } else {
            return Err(JacError::Internal(
                "JacWriter internal writer missing".to_string(),
            ));
        }

        // Update current offset
        self.current_offset += block_size as u64;
        self.metrics.bytes_written += block_size as u64;

        Ok(())
    }

    /// Force flushing of the current (possibly partial) block.
    pub fn flush(&mut self) -> Result<()> {
        self.flush_block()
    }

    /// Encode block data to bytes
    fn encode_block(&self, block_data: &BlockData) -> Result<Vec<u8>> {
        let mut result = Vec::new();

        // Encode block header
        let header_bytes = block_data.header.encode()?;
        result.extend_from_slice(&header_bytes);

        // Write all compressed segments
        for segment in &block_data.segments {
            result.extend_from_slice(segment);
        }

        // Write CRC32C
        result.extend_from_slice(&block_data.crc32c.to_le_bytes());

        Ok(result)
    }

    /// Get current file offset
    fn get_current_offset(&self) -> u64 {
        self.current_offset
    }

    fn finalize(mut self, with_index: bool) -> Result<WriterFinish<W>> {
        // Flush final block
        self.flush_block()?;

        if with_index && !self.block_index.is_empty() {
            // Write index footer
            let index = IndexFooter {
                blocks: self.block_index.clone(),
            };
            let index_bytes = index.encode()?;
            let index_offset = self.get_current_offset();

            if let Some(writer) = self.writer.as_mut() {
                writer.write_all(&index_bytes)?;
            } else {
                return Err(JacError::Internal(
                    "JacWriter internal writer missing".to_string(),
                ));
            }
            self.current_offset += index_bytes.len() as u64;
            self.metrics.bytes_written += index_bytes.len() as u64;

            // Write 8-byte pointer to index (little-endian u64)
            if let Some(writer) = self.writer.as_mut() {
                writer.write_all(&index_offset.to_le_bytes())?;
            } else {
                return Err(JacError::Internal(
                    "JacWriter internal writer missing".to_string(),
                ));
            }
            let pointer_len = std::mem::size_of::<u64>() as u64;
            self.current_offset += pointer_len;
            self.metrics.bytes_written += pointer_len;
        }

        self.finished = true;
        let writer = self
            .writer
            .take()
            .ok_or_else(|| JacError::Internal("JacWriter internal writer missing".to_string()))?;

        Ok(WriterFinish {
            writer,
            metrics: self.metrics,
        })
    }

    /// Finish writing and optionally write index (legacy API).
    pub fn finish(self, with_index: bool) -> Result<W> {
        let finish = self.finalize(with_index)?;
        Ok(finish.writer)
    }

    /// Finish writing and emit an index, returning metrics.
    pub fn finish_with_index(self) -> Result<WriterFinish<W>> {
        self.finalize(true)
    }

    /// Finish writing without emitting an index, returning metrics.
    pub fn finish_without_index(self) -> Result<WriterFinish<W>> {
        self.finalize(false)
    }

    /// Snapshot current metrics without consuming the writer.
    pub fn metrics(&self) -> WriterMetrics {
        self.metrics
    }
}

impl<W: Write> Drop for JacWriter<W> {
    fn drop(&mut self) {
        // In debug mode, warn if finish() wasn't called
        #[cfg(debug_assertions)]
        if !self.finished {
            eprintln!("Warning: JacWriter dropped without calling finish() - data may be lost");
        }
    }
}

/// Finalized writer result containing the underlying writer and metrics snapshot.
pub struct WriterFinish<W> {
    /// The owned writer returned from `JacWriter::finish_*`.
    pub writer: W,
    /// Accumulated metrics describing the write session.
    pub metrics: WriterMetrics,
}

/// Metrics emitted by `JacWriter` to aid progress reporting.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriterMetrics {
    /// Total number of records written (including those in final partial blocks).
    pub records_written: u64,
    /// Total number of blocks emitted to the stream.
    pub blocks_written: u64,
    /// Total bytes written to the underlying writer, including index data.
    pub bytes_written: u64,
}
