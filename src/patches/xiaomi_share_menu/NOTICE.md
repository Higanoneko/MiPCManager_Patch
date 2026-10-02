# Xiaomi Share shell menu third-party notice

The embedded Windows 11 Xiaomi Share shell extension is taken from `YYplus/XiaomiShareShellExt-Minimal` version 1.0.2, commit `bc65af128df9cd516e36fce3c6e9baa4501324d9`, which is based on `cnbluefire/MiDropShellExtForWindows11`.

The imported payload is the GitHub Actions artifact `XiaomiShareShellExt-Minimal-win-x64` (artifact ID `11211739378`, SHA-256 `f05c5b2feaa65e6d3dd8ddb0a65dc02f10ef5ba3ddbd1b27720ed329706d50a4`). Its sparse MSIX identity is signed with a local code-signing certificate and an RFC 3161 timestamp; the source build verifies the timestamp before publishing the artifact.

Core payload SHA-256 values:

- `XiaomiShare.ShellExt.dll`: `2e8d5283158eff16a5f3ba6bc7bf80e43d320f673c4d56cc3740ba1ceb3d5e1b`
- `XiaomiShare.Helper.exe`: `bd877fb520434a21b063955daea5d8bee850e75ca4cf11df3196be0afe1fe252`
- `XiaomiShareShellExt.Identity.msix`: `08a39bd7a10ffe3558ca0ff276cfa0a310a3dedd17d8410b5f9846ca6e90849f`
- `XiaomiShareShellExt.cer`: `5908c5e07f849de736f6143abb08369d939c4b1fbb360a2fcaacfffbe6508682`

## MIT License

Copyright (c) 2024 cnbluefire

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.