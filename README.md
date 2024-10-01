# chunk-parser

![GitHub Package Version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2FStealthOfKing%2Frust-chunk-parser%2Frefs%2Fheads%2Fmaster%2FCargo.toml&query=%24.package.version&prefix=v&label=Rust)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/StealthOfKing/rust-chunk-parser/rust.yml)
![GitHub License](https://img.shields.io/github/license/StealthOfKing/rust-chunk-parser)

Generic chunk parser pattern for parsing [type-length-value] formats.

[type-length-value]: https://en.wikipedia.org/wiki/Type%E2%80%93length%E2%80%93value

## Usage

Define header layout and parser:

```rust
use chunk_parser::prelude::*;

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
