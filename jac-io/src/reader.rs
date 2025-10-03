//! Streaming reader for JAC files

use std::cmp::min;
use std::convert::TryFrom;
use std::io::{Read, Seek, SeekFrom};

use jac_codec::{BlockDecoder, DecompressOpts, FieldSegmentDecoder};
use jac_format::constants::{BLOCK_MAGIC, FILE_MAGIC, INDEX_MAGIC};
use jac_format::varint::decode_uleb128;
use jac_format::{
    BlockHeader, BlockIndexEntry, FieldDirectoryEntry, FileHeader, IndexFooter, JacError, Result,
};
use serde_json::{Map, Value};

/// Streaming reader for JAC containers with optional index support
pub struct JacReader<R: Read + Seek> {
    reader: R,
    file_header: FileHeader,
    index: Option<IndexFooter>,
    index_offset: Option<u64>,
    opts: DecompressOpts,
    strict_mode: bool,
    file_size: u64,
    data_start: u64,
}

impl<R: Read + Seek> JacReader<R> {
    /// Create a new reader using default strict mode (errors stop iteration)
    pub fn new(mut reader: R, opts: DecompressOpts) -> Result<Self> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let (file_header, file_header_len) = Self::read_file_header(&mut reader)?;
        let data_start = file_header_len as u64;
        let after_header_pos = reader.stream_position()?;

        let (index, index_offset) = Self::try_read_index(&mut reader, file_size)?;

        // Restore reader position to just after the file header so sequential reads start correctly
        reader.seek(SeekFrom::Start(after_header_pos))?;

        Ok(Self {
            reader,
            file_header,
            index,
            index_offset,
            opts,
            strict_mode: true,
            file_size,
            data_start,
        })
    }

    /// Create a reader with explicit strict mode behaviour
    pub fn with_strict_mode(reader: R, opts: DecompressOpts, strict: bool) -> Result<Self> {
        let mut this = Self::new(reader, opts)?;
        this.strict_mode = strict;
        Ok(this)
    }

    /// Access the decoded file header
    pub fn file_header(&self) -> &FileHeader {
        &self.file_header
    }

    /// Iterate over blocks in the file
    pub fn blocks(&mut self) -> BlockIterator<'_, R> {
        BlockIterator::new(self)
    }

    /// Stream all records lazily across the file.
    pub fn record_stream(&mut self) -> Result<RecordStream<'_, R>> {
        RecordStream::new(self)
    }

    /// Stream projected values for the supplied field.
    pub fn projection_stream(&mut self, field: String) -> Result<ProjectionStream<'_, R>> {
        ProjectionStream::new(self, field)
    }

    /// Restart projection/record iteration from the first block.
    pub fn restart_projection(&mut self) -> Result<()> {
        self.rewind()
    }

    /// Rewind the underlying reader to the first block boundary.
    pub fn rewind(&mut self) -> Result<()> {
        self.reader.seek(SeekFrom::Start(self.data_start))?;
        Ok(())
    }

    /// Consume the reader and return the underlying stream.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Read the entire block payload into memory
    pub fn read_block_bytes(&mut self, block: &BlockHandle) -> Result<Vec<u8>> {
        self.reader.seek(SeekFrom::Start(block.offset))?;
        let mut buf = vec![0u8; block.size];
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Decode an entire block, verifying CRC and limits
    pub fn decode_block(&mut self, block: &BlockHandle) -> Result<BlockDecoder> {
        let block_bytes = self.read_block_bytes(block)?;
        BlockDecoder::new(&block_bytes, &self.opts)
    }

    /// Project a single field from the supplied block
    pub fn project_field(&mut self, block: &BlockHandle, field: &str) -> Result<FieldIterator> {
        // Validate block integrity first (CRC + layout)
        let block_bytes = self.read_block_bytes(block)?;
        BlockDecoder::new(&block_bytes, &self.opts)?;

        // Locate field entry
        let field_entry = block
            .header
            .fields
            .iter()
            .find(|entry| entry.field_name == field)
            .ok_or_else(|| JacError::Internal(format!("Field '{}' not found in block", field)))?;

        let segment_offset = block.header_size + field_entry.segment_offset;
        let segment_end = segment_offset + field_entry.segment_compressed_len;
        if segment_end > block_bytes.len() {
            return Err(JacError::CorruptBlock);
        }

        let segment_bytes = &block_bytes[segment_offset..segment_end];
        let decoder = FieldSegmentDecoder::new(
            segment_bytes,
            field_entry,
            block.record_count,
            &self.opts.limits,
        )?;

        Ok(FieldIterator {
            decoder,
            record_count: block.record_count,
            current_idx: 0,
        })
    }

    fn data_end(&self) -> u64 {
        self.index_offset.unwrap_or(self.file_size)
    }

    fn read_file_header(reader: &mut R) -> Result<(FileHeader, usize)> {
        let mut header_bytes = Vec::new();

        // Fixed portion of the header (magic + flags + compressor + level)
        let mut fixed = [0u8; 10];
        reader.read_exact(&mut fixed)?;
        if fixed[..3] != FILE_MAGIC[..3] {
            return Err(JacError::InvalidMagic);
        }
        if fixed[3] != FILE_MAGIC[3] {
            return Err(JacError::UnsupportedVersion(fixed[3]));
        }
        header_bytes.extend_from_slice(&fixed);

        // Block size hint
        Self::read_varint_into(reader, &mut header_bytes)?;

        // Metadata length (need value to know how many bytes to read)
        let (metadata_len, _) = Self::read_varint_into(reader, &mut header_bytes)?;

        let mut metadata = vec![0u8; metadata_len as usize];
        reader.read_exact(&mut metadata)?;
        if metadata.iter().all(|&byte| byte == 0) && !metadata.is_empty() {
            return Err(JacError::CorruptHeader);
        }
        header_bytes.extend_from_slice(&metadata);

        let (header, consumed) = FileHeader::decode(&header_bytes)?;
        debug_assert_eq!(consumed, header_bytes.len());
        Ok((header, consumed))
    }

    fn try_read_index(
        reader: &mut R,
        file_size: u64,
    ) -> Result<(Option<IndexFooter>, Option<u64>)> {
        if file_size < 8 {
            return Ok((None, None));
        }

        let current_pos = reader.stream_position()?;
        reader.seek(SeekFrom::End(-8))?;
        let mut pointer_bytes = [0u8; 8];
        reader.read_exact(&mut pointer_bytes)?;
        let index_offset = u64::from_le_bytes(pointer_bytes);

        if index_offset == 0 || index_offset >= file_size.saturating_sub(4) {
            reader.seek(SeekFrom::Start(current_pos))?;
            return Ok((None, None));
        }

        // Index occupies the region [index_offset, file_size - 8)
        if index_offset >= file_size.saturating_sub(8) {
            reader.seek(SeekFrom::Start(current_pos))?;
            return Ok((None, None));
        }

        let index_len = file_size - 8 - index_offset;
        reader.seek(SeekFrom::Start(index_offset))?;
        let mut index_bytes = vec![0u8; index_len as usize];
        reader.read_exact(&mut index_bytes)?;

        // Validate magic before attempting to decode fully
        if index_bytes.len() < 4 || index_bytes[0..4] != INDEX_MAGIC.to_le_bytes() {
            reader.seek(SeekFrom::Start(current_pos))?;
            return Ok((None, None));
        }

        let index = match IndexFooter::decode(&index_bytes) {
            Ok(idx) => idx,
            Err(_) => {
                reader.seek(SeekFrom::Start(current_pos))?;
                return Ok((None, None));
            }
        };

        reader.seek(SeekFrom::Start(current_pos))?;
        Ok((Some(index), Some(index_offset)))
    }

    fn read_block_handle_at(&mut self, offset: u64) -> Result<BlockHandle> {
        if offset < self.data_start || offset >= self.data_end() {
            return Err(JacError::UnexpectedEof);
        }

        self.reader.seek(SeekFrom::Start(offset))?;

        let mut header_bytes = Vec::new();
        let mut magic = [0u8; 4];
        self.reader.read_exact(&mut magic)?;
        if magic != BLOCK_MAGIC.to_le_bytes() {
            return Err(JacError::CorruptBlock);
        }
        header_bytes.extend_from_slice(&magic);

        let (header_len, _) = Self::read_varint_into(&mut self.reader, &mut header_bytes)?;
        let header_len = usize::try_from(header_len).map_err(|_| {
            JacError::LimitExceeded("Header length exceeds platform limits".to_string())
        })?;

        let remaining = self
            .data_end()
            .saturating_sub(offset + header_bytes.len() as u64);
        if header_len as u64 > remaining {
            return Err(JacError::UnexpectedEof);
        }

        let mut rest = vec![0u8; header_len];
        self.reader.read_exact(&mut rest)?;
        header_bytes.extend_from_slice(&rest);

        let (header, consumed) = BlockHeader::decode(&header_bytes, &self.opts.limits)?;
        debug_assert_eq!(consumed, header_bytes.len());

        let segments_len = header.fields.iter().try_fold(0usize, |acc, field| {
            acc.checked_add(field.segment_compressed_len)
                .ok_or_else(|| JacError::LimitExceeded("Block segments size overflow".to_string()))
        })?;

        let header_size = header_bytes.len();
        let block_size = header_size
            .checked_add(segments_len)
            .and_then(|v| v.checked_add(4))
            .ok_or_else(|| JacError::LimitExceeded("Block size overflow".to_string()))?;

        let next_offset = offset
            .checked_add(block_size as u64)
            .ok_or_else(|| JacError::LimitExceeded("Block offset overflow".to_string()))?;

        if next_offset > self.data_end() {
            return Err(JacError::UnexpectedEof);
        }

        // Position reader at the end of this block so streaming iteration can continue
        self.reader.seek(SeekFrom::Start(next_offset))?;

        Ok(BlockHandle {
            offset,
            size: block_size,
            record_count: header.record_count,
            header_size,
            header,
        })
    }

    fn resync_from(&mut self, start_offset: u64) -> Result<Option<u64>> {
        let mut offset = start_offset;
        let data_end = self.data_end();
        if offset + 4 > data_end {
            return Ok(None);
        }

        let magic = BLOCK_MAGIC.to_le_bytes();
        let mut buffer = vec![0u8; 8192];

        while offset + 4 <= data_end {
            self.reader.seek(SeekFrom::Start(offset))?;
            let to_read = min(buffer.len() as u64, data_end - offset) as usize;
            let read_bytes = self.reader.read(&mut buffer[..to_read])?;
            if read_bytes < 4 {
                return Ok(None);
            }

            if let Some(pos) = find_magic(&buffer[..read_bytes], &magic) {
                return Ok(Some(offset + pos as u64));
            }

            if read_bytes == to_read {
                // Advance keeping a 3-byte overlap to avoid missing magic split across reads
                offset = offset.saturating_add(read_bytes as u64 - 3);
            } else {
                break;
            }
        }

        Ok(None)
    }

    fn read_varint_into(reader: &mut R, dest: &mut Vec<u8>) -> Result<(u64, usize)> {
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 1];
            reader.read_exact(&mut buf)?;
            dest.push(buf[0]);
            bytes.push(buf[0]);
            if buf[0] & 0x80 == 0 {
                break;
            }
        }
        let (value, consumed) = decode_uleb128(&bytes)?;
        Ok((value, consumed))
    }
}

/// Iterator over blocks in the file
pub struct BlockIterator<'a, R: Read + Seek> {
    reader: &'a mut JacReader<R>,
    cursor: BlockCursor,
}

impl<'a, R: Read + Seek> BlockIterator<'a, R> {
    fn new(reader: &'a mut JacReader<R>) -> Self {
        let cursor = BlockCursor::new(reader);
        Self { reader, cursor }
    }
}

impl<'a, R: Read + Seek> Iterator for BlockIterator<'a, R> {
    type Item = Result<BlockHandle>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.next_block_handle(&mut self.cursor)
    }
}

#[derive(Clone)]
pub(crate) struct BlockCursor {
    mode: BlockIterMode,
    total_blocks: Option<usize>,
}

impl BlockCursor {
    pub(crate) fn new<R: Read + Seek>(reader: &JacReader<R>) -> Self {
        if let Some(index) = reader.index.clone() {
            Self {
                total_blocks: Some(index.blocks.len()),
                mode: BlockIterMode::Indexed {
                    entries: index.blocks,
                    cursor: 0,
                },
            }
        } else {
            Self {
                total_blocks: None,
                mode: BlockIterMode::Streaming {
                    next_offset: reader.data_start,
                },
            }
        }
    }
}

#[derive(Clone)]
pub(crate) enum BlockIterMode {
    Indexed {
        entries: Vec<BlockIndexEntry>,
        cursor: usize,
    },
    Streaming {
        next_offset: u64,
    },
}

impl<R: Read + Seek> JacReader<R> {
    pub(crate) fn next_block_handle(
        &mut self,
        cursor: &mut BlockCursor,
    ) -> Option<Result<BlockHandle>> {
        match &mut cursor.mode {
            BlockIterMode::Indexed {
                entries,
                cursor: idx,
            } => {
                if *idx >= entries.len() {
                    return None;
                }
                let entry = entries[*idx].clone();
                *idx += 1;

                match self.read_block_handle_at(entry.block_offset) {
                    Ok(handle) => {
                        if handle.size != entry.block_size {
                            return Some(Err(JacError::CorruptBlock));
                        }
                        if handle.record_count != entry.record_count {
                            return Some(Err(JacError::CorruptBlock));
                        }
                        Some(Ok(handle))
                    }
                    Err(err) => Some(Err(err)),
                }
            }
            BlockIterMode::Streaming { next_offset } => {
                let data_end = self.data_end();
                loop {
                    if *next_offset >= data_end {
                        return None;
                    }

                    match self.read_block_handle_at(*next_offset) {
                        Ok(handle) => {
                            *next_offset = handle
                                .offset
                                .checked_add(handle.size as u64)
                                .unwrap_or(data_end);
                            return Some(Ok(handle));
                        }
                        Err(err) => {
                            if self.strict_mode {
                                return Some(Err(err));
                            }
                            let start = next_offset.saturating_add(1);
                            match self.resync_from(start) {
                                Ok(Some(new_offset)) => {
                                    *next_offset = new_offset;
                                    continue;
                                }
                                Ok(None) => return None,
                                Err(resync_err) => return Some(Err(resync_err)),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Handle representing a block within the file
#[derive(Clone)]
pub struct BlockHandle {
    /// File offset of the block start
    pub offset: u64,
    /// Total block size (header + segments + CRC)
    pub size: usize,
    /// Number of records in this block
    pub record_count: usize,
    /// Size of the encoded block header
    pub header_size: usize,
    /// Decoded block header
    pub header: BlockHeader,
}

impl BlockHandle {
    /// Retrieve directory metadata for a field by name
    pub fn field_entry(&self, field: &str) -> Option<&FieldDirectoryEntry> {
        self.header
            .fields
            .iter()
            .find(|entry| entry.field_name == field)
    }
}

/// Iterator over projected field values
pub struct FieldIterator {
    decoder: FieldSegmentDecoder,
    record_count: usize,
    current_idx: usize,
}

impl Iterator for FieldIterator {
    type Item = Result<Option<serde_json::Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_idx >= self.record_count {
            return None;
        }

        let idx = self.current_idx;
        self.current_idx += 1;
        Some(self.decoder.get_value(idx))
    }
}

/// Iterator over complete records streamed block-by-block.
pub struct RecordStream<'a, R: Read + Seek> {
    reader: &'a mut JacReader<R>,
    cursor: BlockCursor,
    blocks_seen: usize,
    total_blocks_hint: Option<usize>,
    current_records: Option<std::vec::IntoIter<Map<String, Value>>>,
}

impl<'a, R: Read + Seek> RecordStream<'a, R> {
    pub(crate) fn new(reader: &'a mut JacReader<R>) -> Result<Self> {
        let cursor = BlockCursor::new(reader);
        let total_blocks_hint = cursor.total_blocks;
        Ok(Self {
            reader,
            cursor,
            blocks_seen: 0,
            total_blocks_hint,
            current_records: None,
        })
    }

    /// Hint for total blocks when an index footer is present; otherwise counts processed blocks.
    pub fn block_count(&self) -> usize {
        self.total_blocks_hint.unwrap_or(self.blocks_seen)
    }

    /// Exact number of blocks decoded so far.
    pub fn blocks_processed(&self) -> usize {
        self.blocks_seen
    }
}

impl<'a, R: Read + Seek> Iterator for RecordStream<'a, R> {
    type Item = Result<Map<String, Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(records) = &mut self.current_records {
                if let Some(record) = records.next() {
                    return Some(Ok(record));
                }
            }

            match self.reader.next_block_handle(&mut self.cursor)? {
                Ok(block) => match self.reader.decode_block(&block) {
                    Ok(decoder) => match decoder.decode_records() {
                        Ok(records) => {
                            self.blocks_seen += 1;
                            self.current_records = Some(records.into_iter());
                        }
                        Err(err) => return Some(Err(err)),
                    },
                    Err(err) => return Some(Err(err)),
                },
                Err(err) => return Some(Err(err)),
            }
        }
    }
}

/// Iterator streaming values for a single field across the file.
pub struct ProjectionStream<'a, R: Read + Seek> {
    reader: &'a mut JacReader<R>,
    field: String,
    cursor: BlockCursor,
    current_iter: Option<FieldIterator>,
}

impl<'a, R: Read + Seek> ProjectionStream<'a, R> {
    pub(crate) fn new(reader: &'a mut JacReader<R>, field: String) -> Result<Self> {
        let cursor = BlockCursor::new(reader);
        Ok(Self {
            reader,
            field,
            cursor,
            current_iter: None,
        })
    }
}

impl<'a, R: Read + Seek> Iterator for ProjectionStream<'a, R> {
    type Item = Result<Option<Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(iter) = &mut self.current_iter {
                if let Some(value) = iter.next() {
                    return Some(value);
                }
            }

            match self.reader.next_block_handle(&mut self.cursor)? {
                Ok(block) => match self.reader.project_field(&block, &self.field) {
                    Ok(iter) => {
                        self.current_iter = Some(iter);
                    }
                    Err(err) => return Some(Err(err)),
                },
                Err(err) => return Some(Err(err)),
            }
        }
    }
}

fn find_magic(buffer: &[u8], magic: &[u8; 4]) -> Option<usize> {
    buffer.windows(4).position(|window| window == magic)
}
