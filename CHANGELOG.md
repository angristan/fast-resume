# [2.0.0](https://github.com/angristan/fast-resume/compare/v1.18.0...v2.0.0) (2026-07-11)


* feat!: complete Rust rewrite ([ac00a92](https://github.com/angristan/fast-resume/commit/ac00a9266868479f4ab7065e07e8885284aa5686))


### Bug Fixes

* address review findings ([952c95d](https://github.com/angristan/fast-resume/commit/952c95d55247c3139c14c2a94ce79722e15a9660))
* allow crush resume from tui ([a5dc6ae](https://github.com/angristan/fast-resume/commit/a5dc6aeb21ef3193297b284221a31cc165133cca))
* allow punctuation in tui search ([121003b](https://github.com/angristan/fast-resume/commit/121003b50b05241d2e322c9fe52ab687a638a8b5))
* apply tui directory filter ([f6fd517](https://github.com/angristan/fast-resume/commit/f6fd517582dbd1fef97f2eb4acfb636c669f63fb))
* avoid deletions after failed adapter scans ([bf7e71d](https://github.com/angristan/fast-resume/commit/bf7e71d1147555d0da122dc0dc0ccf75e1f545f5))
* clear pending search status after results apply ([94aab55](https://github.com/angristan/fast-resume/commit/94aab5524016dde3e56de60f3b20c79538770000))
* coalesce tui search requests ([df20e7a](https://github.com/angristan/fast-resume/commit/df20e7a717e2943fcd60b9c25dbd0b0ae4f3cf14))
* continue incremental scans past malformed files ([22b4a8f](https://github.com/angristan/fast-resume/commit/22b4a8f699dd959d134db0208fa5f93de052f149))
* correct all logo aspects in ghostty ([6cab3c9](https://github.com/angristan/fast-resume/commit/6cab3c95f616a0d8ebe5138ea15560bf2c6384ee))
* correct codex logo aspect in ghostty ([12e346f](https://github.com/angristan/fast-resume/commit/12e346fa20e064b8cb93d4d445c9adba8819362e))
* count filtered cli list matches ([f716b19](https://github.com/angristan/fast-resume/commit/f716b196b85f1f51d1bd28b9b2351374d1a49c06))
* delete custom sessions that stop parsing ([2ae0632](https://github.com/angristan/fast-resume/commit/2ae06329515e29cee3a8c39da45c04af6fe7eff3))
* delete sessions that stop parsing ([dd4407d](https://github.com/angristan/fast-resume/commit/dd4407da2a5add29ff8e9675125c2037d800cd4f))
* ignore modified yolo modal shortcuts ([5549e24](https://github.com/angristan/fast-resume/commit/5549e24335eb44d00eae967ad0cb05d588c2dc80))
* ignore mouse scrolls during modal ([a5449be](https://github.com/angristan/fast-resume/commit/a5449be348c8e8036519c06ad8e19d3df97321cc))
* improve search placeholder ([7061789](https://github.com/angristan/fast-resume/commit/7061789381b0a2d43294acf4e4dd8747405e66b7))
* keep agent names visible in results ([fb3339f](https://github.com/angristan/fast-resume/commit/fb3339f8f2df1823b6ab6c35eceb537c46408a6d))
* keep Codex session identity stable ([87128c1](https://github.com/angristan/fast-resume/commit/87128c1f7858087c2a4fa476c666f824c826ed82))
* keep crush docs on scan errors ([c9acd58](https://github.com/angristan/fast-resume/commit/c9acd581aa0e87dc1e303a83199d8a10bffcba6d))
* keep directory matches in text search ([e710eeb](https://github.com/angristan/fast-resume/commit/e710eeb5c075ae47e6d8d32ddfb7ee600b3a7c22))
* keep no-version-check cli compatibility ([1392da3](https://github.com/angristan/fast-resume/commit/1392da31592e5f6bcda3b53b88daaf3814ca57a0))
* keep opencode docs on sqlite scan errors ([5b4b2e5](https://github.com/angristan/fast-resume/commit/5b4b2e5f46e1fcb005d603bec8cbe8b97a152eca))
* label copy yolo modal accurately ([93faf4a](https://github.com/angristan/fast-resume/commit/93faf4af3e161a90a1808e9606eeec8bfe00877b))
* make yolo modal arrows directional ([b0b1197](https://github.com/angristan/fast-resume/commit/b0b1197ca03a9359471690b12d99470ff7b54638))
* move refresh status to header ([2cea639](https://github.com/angristan/fast-resume/commit/2cea63997bbd5c24490a0bdcb52804d5768a153b))
* normalize logo aspect across terminals ([9b57583](https://github.com/angristan/fast-resume/commit/9b57583a00d5debd79d5af2b94782a2810241c82))
* parse fractional vibe timestamps ([9dc7298](https://github.com/angristan/fast-resume/commit/9dc72984d755d590fc6329179b44fdc4bc014b5f))
* pin yolo modal session ([57cd5c3](https://github.com/angristan/fast-resume/commit/57cd5c3d20eb3a98e5b771aaf72998dddbd50f2b))
* preserve crush activity precision ([d896c97](https://github.com/angristan/fast-resume/commit/d896c975cc55296ef4cf10b8f65299ce473d6ea8))
* preserve icon aspect without terminal query ([615bead](https://github.com/angristan/fast-resume/commit/615beadc9dbef77ffd8ebce426cc3fbf9b459292))
* preserve locked deps during release ([f5a1e54](https://github.com/angristan/fast-resume/commit/f5a1e542a5021f8b695a44528d49422bb4f1ac63))
* preserve navigation during refresh searches ([3171636](https://github.com/angristan/fast-resume/commit/31716363a973aaab41075241886c9889c19127c1))
* preserve opencode content on fetch errors ([e5a0a0a](https://github.com/angristan/fast-resume/commit/e5a0a0a7885bfcbaefe5648d72c91724cbc10135))
* preserve selection across refresh searches ([4a08d3f](https://github.com/angristan/fast-resume/commit/4a08d3ff89f7e12c980c9f5df0130198c3438d02))
* preserve terminal logo aspect ([988fd8b](https://github.com/angristan/fast-resume/commit/988fd8bf41d3fa9f9ff34fca137fb5bf557a6a06))
* preserve tui reload requests ([bc1c861](https://github.com/angristan/fast-resume/commit/bc1c8614d3bc9526b06fd8f7deced296a2749080))
* preserve tui selection identity on refresh ([64c9400](https://github.com/angristan/fast-resume/commit/64c9400c6de87f00c092a6eb16ccc273dc95c9f3))
* redraw tui on resize ([42f9e85](https://github.com/angristan/fast-resume/commit/42f9e854975fb84c18742a5dd40dd5a01820d987))
* refresh legacy opencode message changes ([be4012a](https://github.com/angristan/fast-resume/commit/be4012a65afc0b247454a2ae429164a2f64d3f3e))
* refresh sessions when mtimes decrease ([387c875](https://github.com/angristan/fast-resume/commit/387c87552aeb5647dc98f16bc0c0df6b501b7ad4))
* refresh valid rows after malformed jsonl ([2eafd51](https://github.com/angristan/fast-resume/commit/2eafd51d6019827a574e3715dfd2620b6f3ed19e))
* refresh vibe message changes ([91ef795](https://github.com/angristan/fast-resume/commit/91ef795c3cc206ba068340654bdaf1a6ce30ab7d))
* reject out-of-range date filters ([bb27986](https://github.com/angristan/fast-resume/commit/bb279866cee45e83b61c2155c3412f9b7aab740d))
* remove ignored version check flag ([df3701f](https://github.com/angristan/fast-resume/commit/df3701ff3b07e683ddb6837e71400c21af5b7468))
* render tui status feedback ([bfd79c4](https://github.com/angristan/fast-resume/commit/bfd79c43fd9d6ff42227466b1a74ebf2d06fb88a))
* report tui refresh failures ([2c4407c](https://github.com/angristan/fast-resume/commit/2c4407c865b520bca1802cead917060c8efa128a))
* reset selection for typed searches ([4912389](https://github.com/angristan/fast-resume/commit/4912389e30a46800452bac7e8d7c4693daf48c83))
* reset selection on new tui searches ([10693dd](https://github.com/angristan/fast-resume/commit/10693dd4218e1e4a9963a39ee537fbcb93c4496a))
* restore fuzzy matching for message content ([242bf16](https://github.com/angristan/fast-resume/commit/242bf1601aaa528a77fddce11e592eaa7f40e68c))
* restore Rust index parity regressions ([686d9b9](https://github.com/angristan/fast-resume/commit/686d9b9a59cc34fb364af47559268f8a452ecc5a))
* restore terminal after setup failures ([eba99e0](https://github.com/angristan/fast-resume/commit/eba99e0efc9edab534eee04954c1bc97846f6588))
* restore windows install support ([a68d307](https://github.com/angristan/fast-resume/commit/a68d307b97b863873fc9393bb793b689e49bd681))
* retain crush sessions with malformed parts ([6579a01](https://github.com/angristan/fast-resume/commit/6579a012e3f4c129d8b1338f0961f8953e7a0c38))
* retain legacy opencode content on parse errors ([3080d1e](https://github.com/angristan/fast-resume/commit/3080d1e626a32e8e61eaa5148ea0b671bc28099c))
* retain legacy opencode sessions on scan errors ([205170b](https://github.com/angristan/fast-resume/commit/205170b7e05311197a222044209d969d13e28628))
* retain malformed copilot session identity ([fc7bbe0](https://github.com/angristan/fast-resume/commit/fc7bbe0154de5b687d296bf950b240ecbbef7987))
* retain sessions on parse failures ([f5f50d8](https://github.com/angristan/fast-resume/commit/f5f50d815aa3681c93b310fd974a164e193c39b4))
* scroll long tui search input ([803609a](https://github.com/angristan/fast-resume/commit/803609ad65cc5ab4263089fabc7ed3460304a3ea))
* search refresh results asynchronously ([c57b9cb](https://github.com/angristan/fast-resume/commit/c57b9cb94eb6a831df8e40c390580dafff07878c))
* show friendly empty stats message ([4d15041](https://github.com/angristan/fast-resume/commit/4d150413d3134339f9ba0baa4253edea5755238c))
* speed up refresh scans ([5ff93cd](https://github.com/angristan/fast-resume/commit/5ff93cd9a17dcc4e013f27b0a11c424dc8b1af91))
* track legacy opencode file edits ([3ed4252](https://github.com/angristan/fast-resume/commit/3ed42521aa66af063c2db20d7986bf246f1c71da))
* track opencode sqlite activity mtimes ([f249f37](https://github.com/angristan/fast-resume/commit/f249f3746c7cdd699dcf56b0f04798924bd1b21d))
* use civil-day bounds for yesterday ([37e4ac7](https://github.com/angristan/fast-resume/commit/37e4ac70aaf1a1dfc3d6b5d894685e82589679a3))
* use vscode copilot session ids ([360233c](https://github.com/angristan/fast-resume/commit/360233c7856860b71d10111df7cf9b614998a023))
* wait for tui search results before actions ([e587e0e](https://github.com/angristan/fast-resume/commit/e587e0ec9dba846a9723564045ace10fd044ffa3))


### Features

* add Rust TUI implementation ([77dcf90](https://github.com/angristan/fast-resume/commit/77dcf902fd0eb807303d6138c792f103ae9c7e50))
* highlight Rust search filters ([1bc839c](https://github.com/angristan/fast-resume/commit/1bc839c16c39dc79ee54a8aec8e1456b79283919))
* improve tui preview rendering ([f32498e](https://github.com/angristan/fast-resume/commit/f32498e0261bfe487ce30c4faa254a3af0a1272e))
* make agent filters responsive ([5a412bf](https://github.com/angristan/fast-resume/commit/5a412bf2291c86cd506f359d18036cb3f6d3cec3))
* scroll hovered tui pane with mouse ([311b4ec](https://github.com/angristan/fast-resume/commit/311b4ecf40d6f4e3854b0bfaf0e34b502071e888))
* show session counts in agent filters ([f65de4b](https://github.com/angristan/fast-resume/commit/f65de4b5aaebcb2156ffaf5788ff0f2722463baf))
* stream Rust index refresh progress ([582eb53](https://github.com/angristan/fast-resume/commit/582eb539e0572f36ec6aa2e741d5d505e0b273db))
* sync tui search filters ([4570be6](https://github.com/angristan/fast-resume/commit/4570be68ba66c96f5d329a15371064cb5a399cd9))
* use icons for narrow agent filters ([e92f5d9](https://github.com/angristan/fast-resume/commit/e92f5d90f8359b3f140ad74631f6bff1d49cdc31))


### Performance Improvements

* decouple tui search from input ([e745d03](https://github.com/angristan/fast-resume/commit/e745d036eb67739cd9742252ff1c65a96bfb450b))
* redraw TUI only on changes ([c795041](https://github.com/angristan/fast-resume/commit/c795041785e3d8b645962ec4d0a6497f1cfecfb3))
* reduce Rust refresh overhead ([63fe508](https://github.com/angristan/fast-resume/commit/63fe5081db7e565956e22c78d59d2284b3eda497))
* speed up interactive search ([d6d2c9f](https://github.com/angristan/fast-resume/commit/d6d2c9f9c803eecff62c8f7fcc7a4a544a2029aa))


### BREAKING CHANGES

* the Python implementation was replaced by Rust.

# [1.18.0](https://github.com/angristan/fast-resume/compare/v1.17.3...v1.18.0) (2026-06-03)


### Bug Fixes

* tailor update instructions to install source ([#52](https://github.com/angristan/fast-resume/issues/52)) ([81c83c7](https://github.com/angristan/fast-resume/commit/81c83c7492b0471dd41fbd5c8129f7e7e43c15e8))


### Features

* support renamed coding agent session titles ([#51](https://github.com/angristan/fast-resume/issues/51)) ([9f8f04b](https://github.com/angristan/fast-resume/commit/9f8f04bc93715cc514ad9b0379eb5d0482979289))

## [1.17.3](https://github.com/angristan/fast-resume/compare/v1.17.2...v1.17.3) (2026-05-29)


### Bug Fixes

* update adapter resume commands ([#47](https://github.com/angristan/fast-resume/issues/47)) ([eb4a383](https://github.com/angristan/fast-resume/commit/eb4a3835f4691acd66a5c2bbebbf5503bdbf89e7))

## [1.17.2](https://github.com/angristan/fast-resume/compare/v1.17.1...v1.17.2) (2026-03-05)


### Bug Fixes

* detect Copilot CLI sessions in UUID subdirectories ([787ec65](https://github.com/angristan/fast-resume/commit/787ec65f26951272b33cc85e5d710f8af8b17564)), closes [#27](https://github.com/angristan/fast-resume/issues/27)

## [1.17.1](https://github.com/angristan/fast-resume/compare/v1.17.0...v1.17.1) (2026-03-05)


### Bug Fixes

* handle FileNotFoundError when scanning session files ([d1096a7](https://github.com/angristan/fast-resume/commit/d1096a7bd34654d09b282221830a8324c9bdcc6c)), closes [#29](https://github.com/angristan/fast-resume/issues/29)

# [1.17.0](https://github.com/angristan/fast-resume/compare/v1.16.2...v1.17.0) (2026-02-14)


### Features

* add SQLite support for OpenCode 1.2 storage format ([4b23eeb](https://github.com/angristan/fast-resume/commit/4b23eebf05218c16dfe6d36721c3745c271c514e))

## [1.16.2](https://github.com/angristan/fast-resume/compare/v1.16.1...v1.16.2) (2026-02-13)


### Bug Fixes

* trigger binary build pipeline ([f9043e2](https://github.com/angristan/fast-resume/commit/f9043e26b11525f1a16f2cc4d48119656bbe71fa))

## [1.16.1](https://github.com/angristan/fast-resume/compare/v1.16.0...v1.16.1) (2026-02-13)


### Bug Fixes

* move binary builds into CI workflow and wire up semantic-release outputs ([d5abd55](https://github.com/angristan/fast-resume/commit/d5abd5586342de92acaa773e2afad26d3b8707da))
* wire binary builds into CI workflow ([81d156a](https://github.com/angristan/fast-resume/commit/81d156a4701f8e356c578b83804edbf4aa0cd21a))

# [1.16.0](https://github.com/angristan/fast-resume/compare/v1.15.3...v1.16.0) (2026-02-13)


### Features

* add standalone binary builds for Homebrew distribution ([11cedec](https://github.com/angristan/fast-resume/commit/11cedec9dd8d94f903567edcd8c769642d7bf08b))

## [1.15.3](https://github.com/angristan/fast-resume/compare/v1.15.2...v1.15.3) (2026-02-05)


### Bug Fixes

* pin rich <14.3.2 to prevent freeze with images in iTerm ([08b76a1](https://github.com/angristan/fast-resume/commit/08b76a13f0e58bdcde16211a89d82aa60d162dc6))

## [1.15.2](https://github.com/angristan/fast-resume/compare/v1.15.1...v1.15.2) (2026-02-04)


### Bug Fixes

* **tui:** update selected_session before resume on click ([684259a](https://github.com/angristan/fast-resume/commit/684259a3945a236dd60628976845f5322f56d640))

## [1.15.1](https://github.com/angristan/fast-resume/compare/v1.15.0...v1.15.1) (2026-02-04)


### Bug Fixes

* **tui:** handle Enter key when results table is focused ([5488f48](https://github.com/angristan/fast-resume/commit/5488f483fe96771e77a2aed8c0bc80cae333a48a))
* **vibe:** use --agent auto-approve instead of non-existent --auto-approve flag ([7fb5fae](https://github.com/angristan/fast-resume/commit/7fb5fae1dfdbf6d60aba4a7f82750d744635e8dc))

# [1.15.0](https://github.com/angristan/fast-resume/compare/v1.14.3...v1.15.0) (2026-02-04)


### Features

* **tui:** add pointer cursor styles using Textual 7.4.0 ([29628dc](https://github.com/angristan/fast-resume/commit/29628dcdf1c1c11851354536329931bfcca385ab))

## [1.14.3](https://github.com/angristan/fast-resume/compare/v1.14.2...v1.14.3) (2026-02-04)


### Bug Fixes

* **opencode:** use time.updated for session timestamp instead of time.created ([8200a3d](https://github.com/angristan/fast-resume/commit/8200a3d4925b97f5e07a3c0f141cc80022158563))
* **tui:** show existing sessions immediately during streaming load ([26ae477](https://github.com/angristan/fast-resume/commit/26ae477cace6673f6c6a0f0c636b867c1d52a98b))

## [1.14.2](https://github.com/angristan/fast-resume/compare/v1.14.1...v1.14.2) (2026-02-04)


### Bug Fixes

* **vibe:** update adapter for Vibe 2.0 session format ([#16](https://github.com/angristan/fast-resume/issues/16)) ([85a52aa](https://github.com/angristan/fast-resume/commit/85a52aa44d615fc3530f3533092146ce61fc8e9d))

## [1.14.1](https://github.com/angristan/fast-resume/compare/v1.14.0...v1.14.1) (2026-01-21)


### Bug Fixes

* display full session IDs without truncation ([8fd3e8b](https://github.com/angristan/fast-resume/commit/8fd3e8b0b5298f0fb038c98e8825eca3f0f0b785))

# [1.14.0](https://github.com/angristan/fast-resume/compare/v1.13.1...v1.14.0) (2026-01-20)


### Features

* progressive indexing with on_session callback ([a5af4ff](https://github.com/angristan/fast-resume/commit/a5af4ff8af3dc52554a5fcbe03eaf5c266942ad4))

## [1.13.1](https://github.com/angristan/fast-resume/compare/v1.13.0...v1.13.1) (2026-01-20)


### Bug Fixes

* add missing supports_yolo attribute to adapters ([c8d4e82](https://github.com/angristan/fast-resume/commit/c8d4e824fc88fda9f1641aa46ec9ff5b62029553))

# [1.13.0](https://github.com/angristan/fast-resume/compare/v1.12.8...v1.13.0) (2026-01-19)


### Features

* sort sessions by date by default when no search query ([b0309c9](https://github.com/angristan/fast-resume/commit/b0309c9916cfc907e3964fedbacc32a1474c4046))

## [1.12.8](https://github.com/angristan/fast-resume/compare/v1.12.7...v1.12.8) (2026-01-17)


### Bug Fixes

* anchor age gradient to 24h for green→yellow transition ([3e5029e](https://github.com/angristan/fast-resume/commit/3e5029e9d707899fee271816cf5a898895461135))

## [1.12.7](https://github.com/angristan/fast-resume/compare/v1.12.6...v1.12.7) (2026-01-16)


### Bug Fixes

* sync index before showing stats to ensure accurate data ([f1cf86e](https://github.com/angristan/fast-resume/commit/f1cf86e7eb58d989d647c8642550ef4035676f00))

## [1.12.6](https://github.com/angristan/fast-resume/compare/v1.12.5...v1.12.6) (2026-01-16)


### Performance Improvements

* use Tantivy queries for all filtering instead of post-filtering ([7b3497d](https://github.com/angristan/fast-resume/commit/7b3497d5dce4dbafac00f27b94a79e278e4b45d1))

## [1.12.5](https://github.com/angristan/fast-resume/compare/v1.12.4...v1.12.5) (2026-01-15)


### Performance Improvements

* use binary mode for orjson.loads in JSONL adapters ([70e1d94](https://github.com/angristan/fast-resume/commit/70e1d94706699ed3b5cd7ee09cbf2e6b621b6c8e))

## [1.12.4](https://github.com/angristan/fast-resume/compare/v1.12.3...v1.12.4) (2026-01-01)


### Bug Fixes

* **search:** use hybrid exact+fuzzy search for better ranking ([60d53aa](https://github.com/angristan/fast-resume/commit/60d53aa9413d65a1945ab32c1d6d50632106c0e1)), closes [#533](https://github.com/angristan/fast-resume/issues/533)

## [1.12.3](https://github.com/angristan/fast-resume/compare/v1.12.2...v1.12.3) (2026-01-01)


### Bug Fixes

* **stats:** use timedelta for week_start calculation ([010e257](https://github.com/angristan/fast-resume/commit/010e257b1fa80b0d659a274ccd94a02573ba7c68))

## [1.12.2](https://github.com/angristan/fast-resume/compare/v1.12.1...v1.12.2) (2025-12-31)


### Bug Fixes

* **claude:** use first user message as title instead of summary ([3098d76](https://github.com/angristan/fast-resume/commit/3098d767a35bdacef68350af6139751e8ec1a68b))

## [1.12.1](https://github.com/angristan/fast-resume/compare/v1.12.0...v1.12.1) (2025-12-30)


### Bug Fixes

* **modal:** enable tab key to toggle focus in yolo mode modal ([51609fe](https://github.com/angristan/fast-resume/commit/51609fe5666176f393f93e4ce49121fa272c19b8))

# [1.12.0](https://github.com/angristan/fast-resume/compare/v1.11.0...v1.12.0) (2025-12-30)


### Features

* **filter-bar:** hide agents without sessions ([7d4309f](https://github.com/angristan/fast-resume/commit/7d4309f89808fea898dbe32d0af7c94b6ac61f38))
* **preview:** make preview pane scrollable ([c119dbc](https://github.com/angristan/fast-resume/commit/c119dbc8b4aaa5216596f4878ce736d43aad4dfa))

# [1.11.0](https://github.com/angristan/fast-resume/compare/v1.10.0...v1.11.0) (2025-12-30)


### Features

* **preview:** improve content display with agent icons ([dc884bf](https://github.com/angristan/fast-resume/commit/dc884bfa7fdf4324d8a218621f1de8c64429dd92))

# [1.10.0](https://github.com/angristan/fast-resume/compare/v1.9.0...v1.10.0) (2025-12-30)


### Bug Fixes

* **tui:** guard query_one calls against race condition ([15541b3](https://github.com/angristan/fast-resume/commit/15541b3437e429ac1ce5ab6c5535aaf0d6e1c989))


### Features

* **cli:** add --no-version-check option to disable update checks ([b4fb81e](https://github.com/angristan/fast-resume/commit/b4fb81ebb6fcbe5fcda07af587359cbcb81014bd))

# [1.9.0](https://github.com/angristan/fast-resume/compare/v1.8.1...v1.9.0) (2025-12-29)


### Features

* **tui:** warn on invalid filter values with red strikethrough ([af11531](https://github.com/angristan/fast-resume/commit/af115313e0ffe9a68f9ad7dd8fd1e5af9a94eb64))

## [1.8.1](https://github.com/angristan/fast-resume/compare/v1.8.0...v1.8.1) (2025-12-29)


### Bug Fixes

* **tui:** improve search placeholder with keyword examples ([19bdafd](https://github.com/angristan/fast-resume/commit/19bdafd8e10d4dc30c370b0032c7e27c21948e04))

# [1.8.0](https://github.com/angristan/fast-resume/compare/v1.7.0...v1.8.0) (2025-12-29)


### Features

* **tui:** add keyword autocomplete with Tab to accept ([ade06bc](https://github.com/angristan/fast-resume/commit/ade06bcc70c6a80afee997b12b195dd7bec515e8))

# [1.7.0](https://github.com/angristan/fast-resume/compare/v1.6.0...v1.7.0) (2025-12-29)


### Features

* **tui:** sync filter buttons with agent: keyword in query ([d5f5afe](https://github.com/angristan/fast-resume/commit/d5f5afe9277ce14398af603ecec1d6326e561536))

# [1.6.0](https://github.com/angristan/fast-resume/compare/v1.5.0...v1.6.0) (2025-12-29)


### Features

* **query:** add mixed include/exclude filter support ([1c67926](https://github.com/angristan/fast-resume/commit/1c67926c102986f3876b4ac1bad7943fb56e224a))

# [1.5.0](https://github.com/angristan/fast-resume/compare/v1.4.2...v1.5.0) (2025-12-29)


### Features

* add keyword search syntax (agent:, dir:, date:) ([7f8e2f3](https://github.com/angristan/fast-resume/commit/7f8e2f3428eb429ea72d0fc4e7195d2117a4faff))

## [1.4.2](https://github.com/angristan/fast-resume/compare/v1.4.1...v1.4.2) (2025-12-29)


### Bug Fixes

* **ci:** install deps before running ty type checker ([066d4a1](https://github.com/angristan/fast-resume/commit/066d4a1d9d1d77486630e1ad137d068cc187ff4c))

## [1.4.1](https://github.com/angristan/fast-resume/compare/v1.4.0...v1.4.1) (2025-12-24)


### Bug Fixes

* **index:** use limit=1 instead of limit=0 for agent count query ([d95fba0](https://github.com/angristan/fast-resume/commit/d95fba07a9b559007d1eaf0e4cf8aa1277e99c49))

# [1.4.0](https://github.com/angristan/fast-resume/compare/v1.3.3...v1.4.0) (2025-12-24)


### Features

* **tui:** show filtered session count when agent filter is active ([eb44b63](https://github.com/angristan/fast-resume/commit/eb44b632bb1b8062a283afdb6baa9a4352243991))

## [1.3.3](https://github.com/angristan/fast-resume/compare/v1.3.2...v1.3.3) (2025-12-24)


### Bug Fixes

* **copilot-vscode:** use correct session ID for incremental cache lookup ([f2c8ef8](https://github.com/angristan/fast-resume/commit/f2c8ef834617983c5f13be1b7ab51a8e2ac1e04d))

## [1.3.2](https://github.com/angristan/fast-resume/compare/v1.3.1...v1.3.2) (2025-12-24)


### Bug Fixes

* search with hyphenated agent filter (copilot-vscode, copilot-cli) ([320b706](https://github.com/angristan/fast-resume/commit/320b706b2ebc2b65992020d2b2f6013916556f86))

## [1.3.1](https://github.com/angristan/fast-resume/compare/v1.3.0...v1.3.1) (2025-12-23)


### Bug Fixes

* simplify agent badge names and reduce column width ([590d609](https://github.com/angristan/fast-resume/commit/590d609814bf6f8236479769f18b8f2f14a3f4a7))

# [1.3.0](https://github.com/angristan/fast-resume/compare/v1.2.0...v1.3.0) (2025-12-23)


### Features

* show yolo mode modal on resume for supported agents ([ea8a6e7](https://github.com/angristan/fast-resume/commit/ea8a6e7f19cb5e2df696acc619befe911325f929))

# [1.2.0](https://github.com/angristan/fast-resume/compare/v1.1.1...v1.2.0) (2025-12-22)


### Features

* show version in title bar ([ba745fe](https://github.com/angristan/fast-resume/commit/ba745fe54188686d3b21bf8ca3188d0b2f1213c3))

## [1.1.1](https://github.com/angristan/fast-resume/compare/v1.1.0...v1.1.1) (2025-12-22)


### Bug Fixes

* read version from package metadata instead of hardcoding ([3842c07](https://github.com/angristan/fast-resume/commit/3842c07e6aec0bbc02e5e88e6fdfa33fae90a1c5))

# [1.1.0](https://github.com/angristan/fast-resume/compare/v1.0.0...v1.1.0) (2025-12-22)


### Features

* add update notifications ([8705aea](https://github.com/angristan/fast-resume/commit/8705aeaaeb660478752817cefead9f9e21129f64))

# 1.0.0 (2025-12-22)


### Bug Fixes

* adjust search input layout ([a4bfe2a](https://github.com/angristan/fast-resume/commit/a4bfe2a1d0e8c2c29bdebf1b45f65dda127f3465))
* improve column truncation to use full available width ([8ab8ab5](https://github.com/angristan/fast-resume/commit/8ab8ab5210a0cb8d70ec77245958204e1cb0fb1d))
* preserve renderable styling in DataTable cursor ([51b63db](https://github.com/angristan/fast-resume/commit/51b63dbd376faff07fdd9869a238930dccfb8e14))
* prevent agent name truncation in stats table ([9b943dc](https://github.com/angristan/fast-resume/commit/9b943dcaa3a8b4b389cbc5773770db2ad88424bb))
* prevent click from resuming session ([0748c95](https://github.com/angristan/fast-resume/commit/0748c958d2cbcecd825337e2a5022648fc89d1a4))
* prevent duplicate sessions in index ([9d57297](https://github.com/angristan/fast-resume/commit/9d57297e0e79ebb1e953997e62a0af8f88ec5d63))
* prevent race condition when searching during initial indexing ([8aac433](https://github.com/angristan/fast-resume/commit/8aac433700d29359bcd98d5486e03b85cd6d34c5))
* remove priority from Enter binding to allow command palette theme switching ([a1b54a7](https://github.com/angristan/fast-resume/commit/a1b54a7b26a4677805f1e2b32bd3abe4fd359d50))
* remove session count flicker during search ([d1ecb18](https://github.com/angristan/fast-resume/commit/d1ecb181ed0c1ac3db9d816b9088b264955dd6e7))
* show 'n/a' for sessions without directory ([1f4139c](https://github.com/angristan/fast-resume/commit/1f4139c9fe0c30e90abe38afe6af88305832e83e))
* skip empty sessions with no user prompts ([cad6bff](https://github.com/angristan/fast-resume/commit/cad6bff48386631204302b2caa250920681a023b))
* tone down date column colors for better readability ([0017636](https://github.com/angristan/fast-resume/commit/00176366b83fed52c6a4c5696cf883b7b7dc9dc5))
* update Crush branding to match Charm style ([db1faaf](https://github.com/angristan/fast-resume/commit/db1faafbb5a6055397e68e2778e2b09cce888bda)), closes [#6B51FF](https://github.com/angristan/fast-resume/issues/6B51FF)
* use consistent match highlight style in list and preview ([5ffa1a8](https://github.com/angristan/fast-resume/commit/5ffa1a8f176e6d9862dd655cc2c124ad24b8b085))
* use first user message as title fallback for Claude sessions ([1d912c4](https://github.com/angristan/fast-resume/commit/1d912c41e665e35d09d2f38ef5346d13e55315e4))


### Features

* add --stats CLI option to view index statistics ([3c5a193](https://github.com/angristan/fast-resume/commit/3c5a193db2e4c71eb88821c93550197f5a035938))
* add --yolo flag with auto-detection for Codex/Vibe ([74c3b19](https://github.com/angristan/fast-resume/commit/74c3b19749e58305d03ff5684e39958150764dda))
* add 50ms debounce to search input ([0703e54](https://github.com/angristan/fast-resume/commit/0703e54290913ead8ef96f2306ff8fde56af87c2))
* add agent logo icons with textual-image ([c509e17](https://github.com/angristan/fast-resume/commit/c509e17853778d8426b70a98fbd9493a6d43b28b))
* add Crush (charmbracelet) support ([8104877](https://github.com/angristan/fast-resume/commit/8104877decbfbaf6778141bfb36e616d56fa814e))
* add GitHub Copilot CLI support ([fcd9824](https://github.com/angristan/fast-resume/commit/fcd9824eb81d241a5998fe9d7020dabfd5be7aa9))
* add icons to agent filter tabs ([0b69793](https://github.com/angristan/fast-resume/commit/0b69793dd9b80cd3fd15e153afa0da41236872b2))
* add parse error handling with logging and UI notifications ([e69f60f](https://github.com/angristan/fast-resume/commit/e69f60f2816c19633d231895baf4e8868f8ffd59))
* add pre-commit hooks for ruff and pytest ([6c7f66a](https://github.com/angristan/fast-resume/commit/6c7f66a69295a32d15d449967962b8707f243c61))
* add Turns column showing human interaction count ([7b93475](https://github.com/angristan/fast-resume/commit/7b9347572a1ca4db326b3b965d627d2168a70b6e))
* add VS Code Copilot support, rename copilot to copilot-cli ([ab28ad1](https://github.com/angristan/fast-resume/commit/ab28ad1375b35994fdc4de9a76ffb693de8e2ad0))
* display query time in search box ([0c6e72a](https://github.com/angristan/fast-resume/commit/0c6e72af2aef8dbdb3c5c36d0a33b2eda349cbb3))
* enable full-text search on entire session content ([51904ed](https://github.com/angristan/fast-resume/commit/51904ed5b74903a9fe916056f65900b88063977c))
* enhance stats with raw adapter data and unified message counting ([e4985b7](https://github.com/angristan/fast-resume/commit/e4985b79299d505fd6ef0cf7f19236208e3f391e))
* fzf-style UI with progressive loading ([d2dc59a](https://github.com/angristan/fast-resume/commit/d2dc59a6fadcaacd5ac067c4ad4c43fb63433b10))
* improve preview panel with better formatting ([9fadcd2](https://github.com/angristan/fast-resume/commit/9fadcd2e7a4954c9909add921b191eaa0171a959))
* incremental indexing during streaming for consistent search ([80c542d](https://github.com/angristan/fast-resume/commit/80c542dfc529b37ecb5a77b8685eca9e02e66135))
* init ([a287ccb](https://github.com/angristan/fast-resume/commit/a287ccb5229937287f3da283806d6f5bdd8390cc))
* modernize TUI with compact layout ([79287cd](https://github.com/angristan/fast-resume/commit/79287cd58f445bbe812288bcc9046243a92a7eaa))
* redesign TUI header with branding and pill-style filters ([4589276](https://github.com/angristan/fast-resume/commit/45892765d2d7e66d5ceda3a8f3aa9d4e70d56238))
* remove number key shortcuts for agent filters ([28516ae](https://github.com/angristan/fast-resume/commit/28516ae1c3f4a3586025a712911da2707f764522))
* replace RapidFuzz with Tantivy for faster search ([93536a8](https://github.com/angristan/fast-resume/commit/93536a8899286e76bc131fad694bb3edb8803e49))
* responsive table columns and fix horizontal scroll ([0aefcf5](https://github.com/angristan/fast-resume/commit/0aefcf51d2a9c36383b624e3b18c80fc236d6fd7))
* show toast notification when index is updated ([3299f90](https://github.com/angristan/fast-resume/commit/3299f905ce00d9510c45bd43b8a996f382ea83bd))
* TUI improvements ([cc4d62b](https://github.com/angristan/fast-resume/commit/cc4d62bf5e0eaf1906770289ffffd5d6c29e177a))
* update keybindings for preview toggle and filter cycling ([f45baef](https://github.com/angristan/fast-resume/commit/f45baef160bd54e58516152585990a8b15604b6c))
* use continuous gradient for date colors ([931bb89](https://github.com/angristan/fast-resume/commit/931bb89eb4f8fb35dc2139181f73f394259c1444))


### Performance Improvements

* async loading with smart cache detection ([49b68c7](https://github.com/angristan/fast-resume/commit/49b68c7e4b0dae25fa69baf509ae8e111b94a78e))
* batch filesystem reads in OpenCode adapter ([7c3cfc2](https://github.com/angristan/fast-resume/commit/7c3cfc24a3d2da888f66f42b8f21f1bd1e32f040))
* incremental cache updates with Tantivy as single source of truth ([52b7e75](https://github.com/angristan/fast-resume/commit/52b7e75cf95b0a8dc710d43d7d1b3709225d1e23))
* parallelize Claude session file parsing ([80beda0](https://github.com/angristan/fast-resume/commit/80beda04e40087fcbc40199ec905a9933e6ea4fb))
* switch to orjson for faster JSON parsing ([e61d050](https://github.com/angristan/fast-resume/commit/e61d050759d0eb684ff6cb59a00ed9c745bf8828))
* use JOIN query in Crush adapter to avoid N+1 ([23293db](https://github.com/angristan/fast-resume/commit/23293db733dd917206573c7043f756fd006ba994))
* use one worker per adapter for true parallel loading ([0ae8a31](https://github.com/angristan/fast-resume/commit/0ae8a315120bac594de2791ac08997a8797c6835))


### Reverts

* remove threading from Claude adapter ([edc84a7](https://github.com/angristan/fast-resume/commit/edc84a730df0687eac36796882c418131650bb1d))
