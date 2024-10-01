//! Generic chunk parser pattern.

use std::io::{Read, Seek, SeekFrom, Error as IoError};

pub use fourcc::{FourCC, TypeId};
pub use chunk_parser_derive::chunk_parser;

//------------------------------------------------------------------------------

/// Error type common to all chunk parsers.
#[derive(Debug)]
pub enum Error {
    IoError(IoError), // Forwarded `std::io::Error`.
    ParseError, // General parser error.
    SizeOverflow, // Size type overflow error.
    UnexpectedValue, // Unexpected value.
    Unimplemented, // Unimplemented code paths.
    UnknownChunk // Unknown chunk type.
}

// Wrap `std::io::Error` with `Error`.
impl From<IoError> for Error { fn from(e: IoError) -> Self { Error::IoError(e) } }

/// Error type is always an `Error` enum.
pub type Result<T> = std::result::Result<T, Error>;

/// External chunk parser implementation.
type ParserFn<P, S> = fn(parser: &mut P, header: &<P as Parser>::Header) -> Result<S>;

/// Helper trait maps unsigned types to signed equivalent.
pub trait Signed<T> { type Type; }

impl Signed<u64> for u64 { type Type = i64; }
impl Signed<i64> for i64 { type Type = i64; }

impl Signed<u32> for u32 { type Type = i32; }
impl Signed<i32> for i32 { type Type = i32; }

impl Signed<u16> for u16 { type Type = i16; }
impl Signed<i16> for i16 { type Type = i16; }

impl Signed<u8> for u8 { type Type = i8; }
impl Signed<i8> for i8 { type Type = i8; }

//------------------------------------------------------------------------------

/// The `ParserInner` trait defines access to the internal properties.
pub trait ParserInner {
    /// Internal reader type.
    type Reader: Read + Seek;

    /// Access the internal `struct Parser::reader`.
    fn reader(&mut self) -> &mut Self::Reader;

    /// Get the internal `struct Parser::depth`.
    fn depth(&self) -> usize;

    /// Set the internal `struct Parser::depth`.
    fn set_depth(&mut self, depth: usize);
}

/// The `Parser` trait implements the majority of parser API.
pub trait Parser: ParserInner {
    /// Implementation specific header type.
    type Header;
    type Size: TryFrom<u64> + TryInto<u64> + Signed<Self::Size>;

    /// Parse the implementation specific header.
    fn read_header(&mut self) -> Result<Self::Header> {
        Err(Error::Unimplemented)
    }

    /// Parser function for guessing a file layout.
    fn guesser(&mut self, _header: &Self::Header) -> Result<Self::Size> {
        Err(Error::Unimplemented)
    }

    //--------------------------------------------------------------------------

    /// Read a sized type from the reader.
    #[inline]
    fn read<T: Sized>(&mut self) -> Result<T> where Self::Reader: Reader<T> {
        self.reader().read_typed()
    }

    /// Read a big endian type from the reader.
    #[inline]
    fn read_be<T: Sized>(&mut self) -> Result<T> where Self::Reader: Reader<T> {
        self.reader().read_typed_be()
    }

    /// Seek to a position in the reader.
    #[inline]
    fn seek(&mut self, pos: Self::Size) -> Result<u64> {
        let offset = pos.try_into().map_err(|_|Error::SizeOverflow)?;
        Ok(self.reader().seek(SeekFrom::Start(offset))?)
    }

    /// Skip a number of bytes.
    #[inline]
    fn skip(&mut self, offset: <Self::Size as Signed<Self::Size>>::Type) -> Result<u64> where i64: From<<Self::Size as Signed<Self::Size>>::Type> {
        Ok(self.reader().seek(SeekFrom::Current(offset.try_into().map_err(|_|Error::SizeOverflow)?))?)
    }

    /// Get the current reader position.
    #[inline]
    fn position(&mut self) -> Result<Self::Size> {
        Ok(self.reader().stream_position()?.try_into().map_err(|_|Error::SizeOverflow)?)
    }

    /// Peek at a sized type.
    fn peek<T>(&mut self) -> Result<T> where Self::Reader: Reader<T> {
        let pos = self.reader().stream_position()?;
        let value = self.read()?;
        self.reader().seek(SeekFrom::Start(pos))?;
        Ok(value)
    }

    /// Expect a specific value.
    fn expect<T: Eq>(&mut self, value: &T) -> Result<&mut Self> where Self::Reader: Reader<T> {
        let actual = self.read::<T>()?;
        if &actual == value { Ok(self) }
        else { Err(Error::UnexpectedValue) }
    }

    //--------------------------------------------------------------------------

    /// Internal parser loop.
    fn parse_loop(&mut self, f: ParserFn<Self, Self::Size>, total_size: u64) -> Result<()> {
        loop {
            let header = self.read_header()?;
            let start = self.reader().stream_position()?;
            let size = f(self, &header)?; // the parser function is responsible for parsing the size
            let end = start + TryInto::<u64>::try_into(size).map_err(|_|Error::SizeOverflow)?;
            let pos = self.reader().stream_position()?;
            if pos == total_size { break Ok(()); } // function consumed chunk
            else if pos != end { return Err(Error::ParseError); } // function made a mistake
        }
    }

    /// Parse top level chunk(s) from the reader.
    #[inline]
    fn parse(&mut self, f: ParserFn<Self, Self::Size>) -> Result<()> {
        let total_size = self.reader().seek(SeekFrom::End(0))?;
        self.reader().seek(SeekFrom::Start(0))?;
        self.parse_loop(f, total_size)
    }

    /// Parse nested subchunks within the main parse routine.
    #[inline]
    fn parse_subchunks(&mut self, f: ParserFn<Self, Self::Size>, total_size: Self::Size) -> Result<()> {
        self.set_depth(self.depth() + 1);
        match {
            let pos = self.reader().stream_position()?;
            let size = TryInto::<u64>::try_into(total_size).map_err(|_|Error::SizeOverflow)?;
            self.parse_loop(f, pos + size)
        } {
            res => { self.set_depth(self.depth() - 1); res }
        }
    }
}

//------------------------------------------------------------------------------

/// The `Reader` trait adds a typed read functions to `std::io::Read`.
pub trait Reader<T: Sized> {
    fn read_typed(&mut self) -> Result<T>;
    fn read_typed_be(&mut self) -> Result<T> { Err(Error::Unimplemented) }
}

impl<R> Reader<i32> for R where R: Read {
    fn read_typed(&mut self) -> Result<i32> {
        let mut buf = <[u8;4]>::default();
        self.read_exact(&mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }
    fn read_typed_be(&mut self) -> Result<i32> {
        let mut buf = <[u8;4]>::default();
        self.read_exact(&mut buf)?;
        Ok(i32::from_be_bytes(buf))
    }
}

impl<R> Reader<TypeId> for R where R: Read {
    fn read_typed(&mut self) -> Result<TypeId> {
        let mut typeid = TypeId::default();
        self.read_exact(typeid.as_mut())?;
        Ok(typeid)
    }
}

//------------------------------------------------------------------------------

/// `chunk_parser` prelude.
pub mod prelude {
    pub use super::{FourCC, TypeId};
    pub use super::{Parser, ParserInner};
}

//==============================================================================

#[cfg(test)]
mod tests {
    mod chunk_parser {
        pub use super::super::Error;
        pub use super::super::Result;
    }
    use super::prelude::*;
    use super::chunk_parser;

    // minimal iff parser definition
    #[chunk_parser]
    struct IFFParser;
    impl<R> Parser for IFFParser<R> where R: std::io::Read + std::io::Seek {
        type Header = (TypeId, i32);
        type Size = i32;
        fn read_header(&mut self) -> chunk_parser::Result<Self::Header> {
            Ok((self.read()?, self.read_be()?))
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
    fn parse() {
        let mut iff = IFFParser::buf(DATA);
        iff.parse(|parser, ( typeid, size )| {
            assert_eq!(parser.depth(), 0);
            match typeid {
                b"FORM" => parser.expect(b"TEST")?.skip(size - 4),
                b"TEST" => parser.skip(*size),
                _ => Err(chunk_parser::Error::ParseError)
            }?;
            Ok(*size)
        }).unwrap();
    }
}
