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
}

impl<W: Write> JacWriter<W> {
    /// Create new writer
    pub fn new(mut writer: W, header: FileHeader, opts: CompressOpts) -> Result<Self> {
        // Write file header
        let header_bytes = header.encode()?;
        writer.write_all(&header_bytes)?;

        let block_builder = BlockBuilder::new(opts.clone());

        Ok(Self {
            writer: Some(writer),
            opts,
            block_builder,
            block_index: Vec::new(),
            current_offset: header_bytes.len() as u64,
            finished: false,
        })
    }

    /// Write record to current block
    pub fn write_record(&mut self, rec: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
        // If block is full, flush it first
        if self.block_builder.is_full() {
            self.flush_block()?;
        }

        self.block_builder.add_record(rec.clone())
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

    /// Finish writing and optionally write index
    pub fn finish(mut self, with_index: bool) -> Result<W> {
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

            // Write 8-byte pointer to index (little-endian u64)
            if let Some(writer) = self.writer.as_mut() {
                writer.write_all(&index_offset.to_le_bytes())?;
            } else {
                return Err(JacError::Internal(
                    "JacWriter internal writer missing".to_string(),
                ));
            }
            self.current_offset += std::mem::size_of::<u64>() as u64;
        }

        self.finished = true;
        self.writer
            .take()
            .ok_or_else(|| JacError::Internal("JacWriter internal writer missing".to_string()))
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
