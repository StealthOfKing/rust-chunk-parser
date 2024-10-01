# chunk-parser

Generic chunk parser pattern.

## Usage

Define header layout and parser:

```rust
use chunk_parser::prelude::*;
use chunk_parser::chunk_parser;

#[chunk_parser]
struct IFFParser;

impl<R> Parser for IFFParser<R> where R: std::io::Read + std::io::Seek {
    type Header = (TypeId, i32);
    type Size = i32;
    fn read_header(&mut self) -> chunk_parser::Result<Self::Header> {
        Ok((self.read()?, self.read_be()?))
    }
}
```

Parse the branching structure:

```rust
fn main() {
    let parser = IFFParser::new(reader);
    parser.parse(|parser, ( typeid, size )| {
        match (typeid) {
            b"FORM" => { ... },
            _ => Err(chunk_parser::Error::UnknownChunk)
        }
        Ok(*size)
    })?;
}
```