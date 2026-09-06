# What this package contains, and under what terms

clispeak itself is **MIT OR Apache-2.0**. The files beside this one are the
licences of other people's software that the Linux package redistributes, and
this file says which is which and where the source is.

It exists because the archive we redistribute **carries no licence text of its
own**. That was written down in `docs/releasing.md` and stayed true for
months: we shipped somebody else's GPL software with nothing saying so.

| component | licence | source |
|---|---|---|
| clispeak | MIT OR Apache-2.0 | https://github.com/clispeak/clispeak |
| Piper | MIT | https://github.com/rhasspy/piper |
| piper-phonemize | MIT | https://github.com/rhasspy/piper-phonemize |
| ONNX Runtime | MIT | https://github.com/microsoft/onnxruntime |
| **espeak-ng** | **GPL-3.0-or-later** | https://github.com/espeak-ng/espeak-ng |
| LJ Speech voice model | public domain | https://keithito.com/LJ-Speech-Dataset/ |

## The GPL part, stated plainly

**espeak-ng is GPL-3.0-or-later** and is redistributed inside this package as
part of the Piper release archive, unmodified. Its full licence text is in
`espeak-ng-GPL-3.0.txt` beside this file.

The corresponding source is the espeak-ng release built into the Piper archive
we pin, and it is publicly available at the address above. We redistribute the
upstream binary without modification; anyone wanting the source for the exact
binary here can obtain it from that repository, and we will supply it on
request.

**This is Linux only.** macOS, iOS, Android and Windows all speak through the
platform's own synthesiser and carry none of the above (decisions 96 and 102).

## The voice

`en_US-ljspeech-medium` is trained on the LJ Speech corpus, which is public
domain. That was a deliberate change: the previous default, `en_US-lessac-medium`,
is trained on a corpus licensed for **research purposes only, with
distribution barred outright**, and shipping it would have been a licence
breach rather than a paperwork gap (#24).
