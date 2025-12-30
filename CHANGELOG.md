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
