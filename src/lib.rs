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
type ParserFunction<P, S> = fn(parser: &mut P, header: &<P as Parser>::Header) -> Result<S>;

//------------------------------------------------------------------------------

/// The `ParserCommon` trait implements the majority of parser API.
pub trait ParserCommon {
    /// Internal reader type.
    type Reader: Read + Seek;

    /// Access the internal `struct Parser::reader`.
    fn reader(&mut self) -> &mut Self::Reader;

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
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> where Self: Parser {
        Ok(self.reader().seek(pos)?)
    }

    /// Skip a number of bytes.
    #[inline]
    fn skip(&mut self, size: Self::Size) -> Result<()> where Self: Parser {
        let pos = size.try_into().map_err(|_|Error::SizeOverflow)?;
        self.seek(SeekFrom::Current(pos))?;
        Ok(())
    }

    /// Get the current reader position.
    #[inline]
    fn position(&mut self) -> Result<u64> {
        Ok(self.reader().stream_position()?)
    }

    /// Peek at a sized type.
    fn peek<T>(&mut self) -> Result<T> where Self: Parser, Self::Reader: Reader<T> {
        let pos = self.position()?;
        let value = self.read()?;
        self.seek(SeekFrom::Start(pos))?;
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
    fn parse_loop(&mut self, f: ParserFunction<Self, Self::Size>, total_size: u64) -> Result<()> where Self: Parser {
        loop {
            let header = self.read_header()?;
            let start = self.position()?;
            let size = f(self, &header)?; // the parser function is responsible for parsing the size
            let end = start + TryInto::<u64>::try_into(size).map_err(|_|Error::SizeOverflow)?;
            let pos = self.position()?;
            if pos == total_size { break Ok(()); } // function consumed chunk
            else if pos != end { return Err(Error::ParseError); } // function made a mistake
        }
    }

    /// Parse top level chunk(s) from the reader.
    #[inline]
    fn parse(&mut self, f: ParserFunction<Self, Self::Size>) -> Result<()> where Self: Parser {
        let total_size = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(0))?;
        self.parse_loop(f, total_size)
    }

    /// Parse nested subchunks within the main parse routine.
    #[inline]
    fn parse_subchunks(&mut self, f: ParserFunction<Self, Self::Size>, total_size: Self::Size) -> Result<()> where Self: Parser {
        let pos = self.position()?;
        let size = TryInto::<u64>::try_into(total_size).map_err(|_|Error::SizeOverflow)?;
        self.parse_loop(f, pos + size)
    }
}

/// The `Parser` trait adds implementation specific header parsing.
pub trait Parser: ParserCommon {
    /// Implementation specific header type.
    type Header;
    type Size: TryInto<u64> + TryInto<i64>;

    /// Parse the implementation specific header.
    fn read_header(&mut self) -> Result<Self::Header> {
        Err(Error::Unimplemented)
    }

    /// Parser function for guessing a file layout.
    fn guesser(&mut self, _header: &Self::Header) -> Result<Self::Size> {
        Err(Error::Unimplemented)
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
    pub use super::{Parser, ParserCommon};
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
            match typeid {
                b"FORM" => parser.expect(b"TEST")?.skip(size - 4),
                b"TEST" => parser.skip(*size),
                _ => Err(chunk_parser::Error::ParseError)
            }?;
            Ok(*size)
        }).unwrap();
    }
}
