//! Generic chunk parser pattern.

use std::io::{Read, Seek, SeekFrom, Error as IoError};
use std::mem::MaybeUninit;
use std::path::PathBuf;

use num::traits::PrimInt;

pub use fourcc::{FourCC, TypeId};
pub use chunk_parser_derive::chunk_parser;

//------------------------------------------------------------------------------

/// Error type common to all chunk parsers.
#[derive(Debug)]
pub enum Error {
    IoError(IoError), // Forwarded `std::io::Error`.
    ParseError, // General parser error.
    SizeOverflow, // Size type overflow error.
    Unimplemented, // Unimplemented code paths.
    UnknownChunk // Unknown chunk type.
}

// Wrap `std::io::Error` with `Error`.
impl From<IoError> for Error { fn from(e: IoError) -> Self { Error::IoError(e) } }

/// Error type is always an `Error` enum.
pub type Result<T> = std::result::Result<T, Error>;

//------------------------------------------------------------------------------

/// The `ParserReader` trait defines access to the inner reader.
///
/// All parsing traits operate on an internal reader type `<R>`, usually
/// implementing a combination of `std::io::Read` and `Seek`. This generic trait
/// adds mutable borrowed access and allows reclaiming the reader for any type
/// `<R>` that implements `DummyReader<R>`.
pub trait ParserReader<R> {
    /// Access the inner reader.
    fn reader(&mut self) -> &mut R;

    // Replace with a dummy file.
    fn take_reader(&mut self) -> R where R: DummyReader {
        std::mem::replace(self.reader(), R::dummy())
    }
}

/// The `ParserSeek` trait implements positional API.
///
/// Seek operations are an essential part of efficiently parsing block chunk
/// file formats. `ParserSeek` varies slightly from `std::io::Seek` due to the
/// bespoke design of `ChunkParser` for parsing header prefixed chunks where
/// the length is guaranteed.
pub trait ParserSeek<R: Seek>: ParserReader<R> {
    /// Seek to a position in the reader.
    #[inline] fn seek(&mut self, offset: u64) -> Result<u64> {
        let pos = SeekFrom::Start(offset);
        Ok( self.reader().seek(pos)? )
    }

    /// Skip a number of bytes.
    #[inline] fn skip(&mut self, offset: u64) -> Result<u64> {
        let pos = SeekFrom::Current(offset as i64);
        self.reader().seek(pos)?;
        Ok( offset )
    }

    /// Rewind a number of bytes.
    #[inline] fn rewind(&mut self, offset: u64) -> Result<u64> {
        let pos = SeekFrom::Current(-(offset as i64));
        Ok( self.reader().seek(pos)? )
    }

    /// Get the current reader position.
    #[inline] fn position(&mut self) -> Result<u64>
        { Ok( self.reader().stream_position()? ) }
}

/// The `ParserDepth` trait can be used to track depth.
///
/// Many block chunk file formats use nested groups to provide hierarchical
/// structure to their data. This trait adds API for tracking the parser depth,
/// but must be manually implemented in the parser loop.
pub trait ParserDepth {
    /// Access the inner depth property.
    fn inner_depth(&mut self) -> &mut u8;

    /// Get the current parser depth.
    fn depth(&mut self) -> u8 { *self.inner_depth() }

    /// Increment the parser depth.
    #[inline] fn push(&mut self) { *self.inner_depth() += 1; }

    /// Decrement the parser depth.
    #[inline] fn pop(&mut self) { *self.inner_depth() -= 1; }
}

/// The `ParserPath` trait accesses the the file path.
///
/// It can be useful to know where a resource was loaded from, not least of all
/// for debugging purposes. This trait adds access to the original location used
/// to create the parser.
pub trait ParserPath {
    /// Access the parser file path.
    fn path(&self) -> &PathBuf;
}

/// The `ParserRead` trait provides typed read API.
///
/// The `ParserRead` API uses `read_uninit()` to read any sized type directly
/// into uninitialised memory.
///
/// Block chunk file formats like IFF often layout their data in such a way that
/// it can be directly loaded into packed struct layouts in memory. This can
/// reduce operational overhead when parsing data by grouping together many
/// smaller variables. Use `ParserRead` in conjunction with packed struct types
/// like those defined in `esm_bindings`.
pub trait ParserRead<R: Read>: ParserReader<R> {
    /// Read a sized type from the reader into uninitialised memory.
    #[inline] fn read<T: Sized>(&mut self) -> Result<T>
        { self.reader().read_uninit() }

    /// Big endian read for all primitive integer types.
    #[inline] fn read_be<T: PrimInt>(&mut self) -> Result<T>
        { Ok( T::swap_bytes(self.reader().read_uninit()?) ) }
}

//------------------------------------------------------------------------------

/// The `ReaderUninit` trait adds a typed read function.
pub trait ReaderUninit<T: Sized> {
    fn read_uninit(&mut self) -> Result<T>;
}

// Blanket implementation of typed read.
impl<R: Read, T: Sized> ReaderUninit<T> for R {
    fn read_uninit(&mut self) -> Result<T> {
        let mut uninit = MaybeUninit::<T>::uninit(); // allocate memory
        Ok( unsafe { // read directly into pointer
            let ptr = uninit.as_mut_ptr();
            self.read_exact(std::slice::from_raw_parts_mut(ptr as *mut u8, std::mem::size_of::<T>()))?;
            uninit.assume_init() // confirm initialisation
        } )
    }
}

//------------------------------------------------------------------------------

/// Dummy constructor trait for reader types.
pub trait DummyReader {
    fn dummy() -> Self;
}

impl DummyReader for std::io::BufReader<std::fs::File> {
    fn dummy() -> std::io::BufReader<std::fs::File> {
        let file = std::fs::File::open("dummy.txt").unwrap();
        std::io::BufReader::new(file)
    }
}

//------------------------------------------------------------------------------

/// The `HeaderParser` trait defines unique header parsing logic.
pub trait HeaderParser<H> {
    fn header(&mut self) -> Result<H>;
}

/// Signature for parser closures.
pub type ParserFn<P,H> = fn(parser: &mut P, header: &H) -> Result<u64>;

/// The `ChunkParser` trait defines the inner parser loop.
pub trait ChunkParser<R: Read + Seek>: ParserRead<R> + ParserDepth {
    /// Internal parser loop.
    fn parse_loop<H>(&mut self, f: ParserFn<Self,H>, total_size: u64) -> Result<()> where Self: HeaderParser<H> {
        loop {
            let header = self.header()?;
            let start = self.reader().stream_position()?;
            let size = f(self, &header)?; // the parser function is responsible for parsing the size
            let end = start + size;
            let pos = self.reader().stream_position()?;
            if pos == total_size { break Ok(()) } // function consumed chunk
            else if pos != end { break Err(Error::ParseError) } // function made a mistake
        }
    }

    /// Parse top level chunk(s) from the reader.
    #[inline]
    fn parse<H>(&mut self, f: ParserFn<Self,H>) -> Result<()> where Self: HeaderParser<H> {
        let total_size = self.reader().seek(SeekFrom::End(0))?;
        self.reader().seek(SeekFrom::Start(0))?;
        self.parse_loop(f, total_size)
    }

    /// Parse nested subchunks within the main parse routine.
    #[inline]
    fn subchunks<H>(&mut self, f: ParserFn<Self,H>, total_size: u64) -> Result<()> where Self: HeaderParser<H> {
        self.push();
        match {
            let pos = self.reader().stream_position()?;
            self.parse_loop(f, pos + total_size)
        } {
            res => { self.pop(); res }
        }
    }
}

//------------------------------------------------------------------------------

/// `chunk_parser` prelude.
pub mod prelude {
    pub use super::{FourCC, TypeId};
    pub use super::{
        HeaderParser, ChunkParser,
        ParserReader, ParserRead, ParserSeek,
        ParserDepth, ParserPath,
        ParserFn
    };
    pub use super::chunk_parser;
}

//==============================================================================

#[cfg(test)]
mod tests {
    mod chunk_parser {
        pub use super::super::Error;
        pub use super::super::Result;
    }
    use super::prelude::*;
    use chunk_parser::{Error, Result};

    // full iff parser definition without macro
    use std::io::{Read, Seek};
    struct IFFParserFull<R> { reader: R, depth: u8 }
    impl<R: Read> IFFParserFull<R> { fn new(reader: R) -> IFFParserFull<R> { IFFParserFull{ reader, depth: 0 } } }
    impl<R> ParserReader<R> for IFFParserFull<R> { fn reader(&mut self) -> &mut R { &mut self.reader } }
    impl<R: Seek> ParserSeek<R> for IFFParserFull<R> {}
    impl<R: Read> ParserRead<R> for IFFParserFull<R> {}
    impl<R> ParserDepth for IFFParserFull<R> { fn inner_depth(&mut self) -> &mut u8 { &mut self.depth } }
    impl<R: Read + Seek> ChunkParser<R> for IFFParserFull<R> {}

    // Simple header definition.
    struct IFFHeader { typeid: TypeId, length: u32 }
    impl<R: Read> HeaderParser<IFFHeader> for IFFParserFull<R> {
        fn header(&mut self) -> Result<IFFHeader>
            { Ok( IFFHeader { typeid: self.read()?, length: self.read_be()? } ) }
    }

    // minimal iff parser definition with macro
    #[chunk_parser]
    struct IFFParser;
    impl<R: Read> HeaderParser<IFFHeader> for IFFParser<R> {
        fn header(&mut self) -> Result<IFFHeader>
            { Ok( IFFHeader { typeid: self.read()?, length: self.read_be()? } ) }
    }

    // minimal custom parser loop
    #[chunk_parser(custom,depth)]
    struct IFFParserCustom;
    impl<R: Read> HeaderParser<IFFHeader> for IFFParserCustom<R> {
        fn header(&mut self) -> Result<IFFHeader>
            { Ok( IFFHeader { typeid: self.read()?, length: self.read_be::<u32>()? - 8 } ) }
    }
    impl<R: std::io::Read + std::io::Seek> ChunkParser<R> for IFFParserCustom<R> {
        fn parse_loop<H>(&mut self, f: ParserFn<Self,H>, total_size: u64) -> Result<()> where Self: HeaderParser<H> {
            self.push();
            match loop {
                let header = self.header()?;
                let start = self.reader().stream_position()?;
                let size = f(self, &header)? + 8; // the parser function is responsible for parsing the size
                let end = start + size;
                let pos = self.reader().stream_position()?;
                if pos == total_size { break Ok(()) } // function consumed chunk
                else if pos != end { break Err(Error::ParseError) } // function made a mistake
            } {
                res => { self.pop(); res }
            }
        }
    }

    // nonsense data to test basic functionality
    const DATA: &[u8;24] = &[
        // FORM chunk (24 bytes)
        0x46, 0x4f, 0x52, 0x4d, // "FORM" chunk typeid
        0x00, 0x00, 0x00, 0x10, // Chunk size (16 bytes)
        0x54, 0x45, 0x53, 0x54, // Subchunk typeid ("TEST")

        // TEST chunk (12 bytes)
        0x54, 0x45, 0x53, 0x54, // "TEST" chunk typid
        0x00, 0x00, 0x00, 0x04, // Chunk size (4 bytes)
        0x01, 0x02, 0x03, 0x04, // Test data
    ];

    #[test]
    fn without_macro() -> Result<()> {
        let mut cursor = std::io::Cursor::new(DATA);
        let mut iff = IFFParserFull::new(&mut cursor);
        iff.parse(|parser, header| {
            if &header.typeid != b"FORM" { panic!(); }
            parser.skip(header.length as u64)
        })
    }

    #[test]
    fn with_macro() -> Result<()> {
        let mut cursor = std::io::Cursor::new(DATA);
        let mut iff = IFFParser::new(&mut cursor);
        iff.parse(|parser, header| parser.skip(header.length as u64))
    }

    #[test]
    fn cursor() -> Result<()> {
        IFFParser::cursor(DATA).parse(|parser, header| parser.skip(header.length as u64))
    }

    #[test]
    fn custom() -> Result<()> {
        let mut iff = IFFParserCustom::cursor(DATA);
        iff.parse(|parser, header| parser.skip(header.length as u64 + 8))
    }
}
