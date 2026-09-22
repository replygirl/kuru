# Third-party notices

Kuru's native search implementation uses the following upstream components
from [BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep):

- `ignore` 0.4.33
- `grep-regex` 0.1.14
- `grep-searcher` 0.1.17

Each is licensed under either the MIT License or the Unlicense, at the user's
option. The source distributions retain both license texts; Kuru preserves that
choice and includes this notice with its connector source distribution.

Kuru's offline input sizing embeds `tiktoken-rs` 0.12.0 from
[zurawiki/tiktoken-rs](https://github.com/zurawiki/tiktoken-rs/tree/v0.12.0)
under the MIT License. Its embedded `o200k_base.tiktoken` asset matches the
[OpenAI tiktoken](https://github.com/openai/tiktoken) public asset SHA-256
`446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`
and carries OpenAI's MIT notice. The MIT terms below apply to these components
as well as to ripgrep's MIT-licensed option.

## MIT License

Copyright (c) 2017 Andrew Gallant
Copyright (c) 2023 Roger Zurawicki
Copyright (c) 2022 OpenAI, Shantanu Jain

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions: The above copyright notice and this
permission notice shall be included in all copies or substantial portions of
the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO
EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

## Unlicense

This is free and unencumbered software released into the public domain. Anyone
is free to copy, modify, publish, use, compile, sell, or distribute this
software, either in source code form or as a compiled binary, for any purpose,
commercial or non-commercial, and by any means. THE SOFTWARE IS PROVIDED "AS
IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE
AND NONINFRINGEMENT. In no event shall the authors be liable for any claim,
damages or other liability arising from, out of or in connection with the
software or its use.
