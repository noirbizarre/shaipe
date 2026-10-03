# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0 - 2026-10-03

### 💫 Features

- **acp** Request a model from the agent's own selector, and pick one mid-session - ([19d97ba](https://github.com/noirbizarre/shaipe/commit/19d97ba4758b8da927bfa60fb33bc71d3b2b57b0))
- **acp** Deny the agent's editing tools through its environment - ([b2915b9](https://github.com/noirbizarre/shaipe/commit/b2915b99bd132c662183bdde7fc65dc6b7a8a15e))
- **acp** Answer permission requests by what the tool would do - ([1da868d](https://github.com/noirbizarre/shaipe/commit/1da868d3ea72fb0f00d6657456e65e9b8fb00a87))
- **acp** Drive an agent the user already has - ([6961d45](https://github.com/noirbizarre/shaipe/commit/6961d454f8ea8cf5e2f1b8eb16eb9780f04eda37))
- **analysis** Measure gradients, transparency and strokes ([#43](https://github.com/noirbizarre/shaipe/issues/43)) - ([9fd6d5c](https://github.com/noirbizarre/shaipe/commit/9fd6d5cf26f484fd04c1be33294e83b2e4e91323))
- **cli** Let a project declare its own render output directory - ([8695469](https://github.com/noirbizarre/shaipe/commit/8695469c64aba634c7d11734c5da55f2b920027f))
- **cli** Implement `shaipe init` - ([973a84b](https://github.com/noirbizarre/shaipe/commit/973a84b8387b20cd70ac37998a4b7e5be57c571e))
- **compare** Report appearance differences apart from geometry ([#45](https://github.com/noirbizarre/shaipe/issues/45)) - ([9638c4b](https://github.com/noirbizarre/shaipe/commit/9638c4bd59eb84bd12c31bb51f6e737eff5eea0a))
- **fonts** Let a font be declared by a checksum-pinned URL - ([54f0ee3](https://github.com/noirbizarre/shaipe/commit/54f0ee3115734583aa79541adf316068f8bc3313))
- **mcp** Reconstruction instructions and tool-use policy ([#22](https://github.com/noirbizarre/shaipe/issues/22)) - ([be8d08b](https://github.com/noirbizarre/shaipe/commit/be8d08bd9767feb327b28d16ef8b8ce922de3e62))
- **mcp** Serve a live workspace over a socket, reached by a bridge - ([fe0cdab](https://github.com/noirbizarre/shaipe/commit/fe0cdab7cf2e9c82c7c974ad592653d01d2025d8))
- **mcp** Serve the tools over stdio - ([5e46d62](https://github.com/noirbizarre/shaipe/commit/5e46d6273ac84f7b553b3da66356c5de2d7af5a4))
- **palette** Bind artwork to palette entries so editing a colour restyles it - ([5756831](https://github.com/noirbizarre/shaipe/commit/57568314484a037685cc904e42ffe15e1514d4d6))
- **tools** Make the reference, render and write tools one coherent contract ([#23](https://github.com/noirbizarre/shaipe/issues/23)) - ([e79c750](https://github.com/noirbizarre/shaipe/commit/e79c750201af77cb76b744feb7a64a5e7caa25fe))
- **tools** Name the reconstruction workflow's phases and recommend a strategy ([#21](https://github.com/noirbizarre/shaipe/issues/21)) - ([281259c](https://github.com/noirbizarre/shaipe/commit/281259cccf438f4de04d7b99c32b1d5842f6c2a1))
- **tools** Trace colour regions, and measure what a trace produced ([#20](https://github.com/noirbizarre/shaipe/issues/20)) - ([6aad05c](https://github.com/noirbizarre/shaipe/commit/6aad05c46076e4d4d9ea2fcddbee7d9fea32f994))
- **tools** Compare a reference against a rendered variant, with a visual diff ([#19](https://github.com/noirbizarre/shaipe/issues/19)) - ([d8957f6](https://github.com/noirbizarre/shaipe/commit/d8957f603d055094be2eb4d2ac89bb1cf7a2b4d7))
- **tools** Measure a reference image into structured facts deterministically ([#18](https://github.com/noirbizarre/shaipe/issues/18)) - ([45fed53](https://github.com/noirbizarre/shaipe/commit/45fed537e6b19808c924fde197f1eabd9f9ea8d0))
- **tools** Trace a reference image into vector paths deterministically - ([47f52de](https://github.com/noirbizarre/shaipe/commit/47f52de839e83ef6ea9759d0f3758132a7d77961))
- **tools** Read a reference image back to an agent, and attach one at init - ([92f10bb](https://github.com/noirbizarre/shaipe/commit/92f10bbfb498fd10f4f282b90f6914b0dd3df3f9))
- **tools** Attach and inspect references - ([85dbfd1](https://github.com/noirbizarre/shaipe/commit/85dbfd1454bca592c279668d5dd90c0921ab20a5))
- **tools** Mutating tools for the palette, a variant and generation - ([9fc14a4](https://github.com/noirbizarre/shaipe/commit/9fc14a4a8f34ef37337f453a458e429314b85c85))
- **tools** Reach a project by message rather than by reference - ([29d3c7f](https://github.com/noirbizarre/shaipe/commit/29d3c7f9a80c8ec2a0fe2b22c70e824f5866971d))
- **tools** Add get_svg, write_svg and render_grid - ([ceaba12](https://github.com/noirbizarre/shaipe/commit/ceaba127342ee9aea8e8a2d49ac2674b18a394c3))
- **tui** Remember the last chosen model, and prefer one with vision - ([7f9e67e](https://github.com/noirbizarre/shaipe/commit/7f9e67e37839f276f0436cf2dd874e5f2846aa5c))
- **tui** Attach and browse inspiration images alongside the prompt - ([8f3a466](https://github.com/noirbizarre/shaipe/commit/8f3a4660cd683dfd044f864968e08720c927a49b))
- **tui** A variant's preview fills the pane, a render specification keeps its declared size - ([ea193c1](https://github.com/noirbizarre/shaipe/commit/ea193c137231b274046363a61c33824b495b4359))
- **tui** Render the transcript's Markdown instead of its characters - ([2495bb6](https://github.com/noirbizarre/shaipe/commit/2495bb69079db6cb576092d60a278f9820d07443))
- **tui** Add, remove and reorder variants and render specifications - ([807c135](https://github.com/noirbizarre/shaipe/commit/807c1353af89517f161167a67974095ce9768cd1))
- **tui** A colour picker for the palette pane - ([911835c](https://github.com/noirbizarre/shaipe/commit/911835c79479de1479e9196a42b55c14047170e5))
- **tui** A toolbar, tabbed previews and panes edited in place - ([da6beec](https://github.com/noirbizarre/shaipe/commit/da6beec9384f68af779030fd36385de889566ef8))
- **tui** Give the agent's log a column of its own - ([ab86966](https://github.com/noirbizarre/shaipe/commit/ab869667aebf2162aea1ee5fb2b4d1477ecb26e3))
- **tui** The prompt is the instruction, and `a` sends it - ([f669ab7](https://github.com/noirbizarre/shaipe/commit/f669ab7071a44cf7aa498ca8c86770851b5a9f37))
- **tui** Mark a change on disk apart from unsaved work - ([ed1a8bf](https://github.com/noirbizarre/shaipe/commit/ed1a8bfdde44119d6cbe1a0c121d36ddbf85bdb1))
- **tui** Notice the file changing underneath the workspace - ([258682a](https://github.com/noirbizarre/shaipe/commit/258682a0f6841297bbda156fec6c389ca063b6fb))
- **tui** Edit the prompt, save the project, guard unsaved work - ([513bd8b](https://github.com/noirbizarre/shaipe/commit/513bd8bf81c87015054ad4bc43dc0c277fe0f870))
- **tui** Render off the drawing thread, with a spinner - ([b7c4692](https://github.com/noirbizarre/shaipe/commit/b7c4692cd0ddb513681efbe367e77d954106cc2b))
- **tui** Mouse support, and give the focused pane the column - ([2f1ed20](https://github.com/noirbizarre/shaipe/commit/2f1ed200eceb217c47eac7953d5f3a72683d145a))
- **vectorize** Keep gradients and translucent fills as one painted path ([#44](https://github.com/noirbizarre/shaipe/issues/44)) - ([7f0e5e5](https://github.com/noirbizarre/shaipe/commit/7f0e5e503dbf905f767b511f24fb288d787155b6))
- Add structured component and composition understanding ([#50](https://github.com/noirbizarre/shaipe/issues/50)) ([#58](https://github.com/noirbizarre/shaipe/issues/58)) - ([5591c69](https://github.com/noirbizarre/shaipe/commit/5591c690d799aab518996c9e77d8716b38a9a9b6))
- Improve typography recognition and reconstruction ([#49](https://github.com/noirbizarre/shaipe/issues/49)) ([#57](https://github.com/noirbizarre/shaipe/issues/57)) - ([e493d05](https://github.com/noirbizarre/shaipe/commit/e493d059c1f9892be5955706f4bf704db560de21))
- Tool registry, dogfooding artwork and architecture guards - ([d43d434](https://github.com/noirbizarre/shaipe/commit/d43d4342188cdb068f4bb624b48503408c55f477))
- CLI, terminal preview and interactive workspace - ([430e3e1](https://github.com/noirbizarre/shaipe/commit/430e3e107d031c878fce0f67b405c81f2e1bf0e8))
- Project model and deterministic renderer - ([ac9b382](https://github.com/noirbizarre/shaipe/commit/ac9b3821564a4e39dc3c76c1cdc6aac3e73899b2))

### 🐛 Bug Fixes

- **acp** Report a successful startup model switch, not just a mid-session one - ([6981347](https://github.com/noirbizarre/shaipe/commit/698134713bce1cc4ef17a0b4de43d1241683a939))
- **acp** Make which() portable, and stop hardcoding unix-only test fixtures - ([83ac5f2](https://github.com/noirbizarre/shaipe/commit/83ac5f2a138d7dba01321d15c4173729bc232167))
- **ci** Bump the pinned mise version, prek is unusable on 2026.7.18 - ([23fb32c](https://github.com/noirbizarre/shaipe/commit/23fb32c391412be7b9e35c7ae4b2a7c58e014597))
- **cli** Blame stdout, not a file, when writing a report fails - ([a8d42e9](https://github.com/noirbizarre/shaipe/commit/a8d42e97696bb9c32c48122165cc346332804d57))
- **cli** Store shaipe init's --source/--inspiration relative to the project - ([aa01d9f](https://github.com/noirbizarre/shaipe/commit/aa01d9f1561a3be247d96c2598891a8526be6e02))
- **error** Give every diagnostic an action and correct three that misled - ([319ea6b](https://github.com/noirbizarre/shaipe/commit/319ea6ba83493b394fd1bb90863796e39311d1f9))
- **logo** Declare a committed Fira Sans font for the wordmark - ([7c45414](https://github.com/noirbizarre/shaipe/commit/7c45414ad5513dbd30a56209c7645125169b6aa1))
- **mcp** Follow rmcp 3.4.1's ServerInfo -> ServerConfig rename - ([7668820](https://github.com/noirbizarre/shaipe/commit/766882057394b90b1b3b8f47614236a335e1f6f1))
- **mcp** Gate bridge.rs's unix-socket tests behind cfg(unix) - ([c1179f9](https://github.com/noirbizarre/shaipe/commit/c1179f9ff6914048d8f09e1b3e206311c595b44a))
- **mise** Regenerate mise.lock with per-platform lockfile entries - ([f11254b](https://github.com/noirbizarre/shaipe/commit/f11254b52d975b410d16c32cd6def7615ff7137c))
- **palette** Avoid chunks_exact to satisfy a newer clippy lint - ([b0d64b0](https://github.com/noirbizarre/shaipe/commit/b0d64b0fd31db94722f1d7231b887a2103833e13))
- **preview** Skip the terminal query outright when stdio is not a terminal - ([e12347a](https://github.com/noirbizarre/shaipe/commit/e12347ac4ca53ed151db078108a4b57b47309372))
- **preview** Send the terminal capability query at all - ([4254c90](https://github.com/noirbizarre/shaipe/commit/4254c9017445141bae219cc5aecc26511114084c))
- **project** Detect a directory by is_dir(), not by the read error's kind - ([ffb5298](https://github.com/noirbizarre/shaipe/commit/ffb5298b40f121dcab930604699b960492be3925))
- **project** Give a fresh project a comfortable display size - ([a80f8c4](https://github.com/noirbizarre/shaipe/commit/a80f8c4fa1e49bc75ef0db9972ae6d181581e492))
- **repo** Force LF line endings on checkout - ([b349963](https://github.com/noirbizarre/shaipe/commit/b3499634a3af4484c9954bd8c5feabf4a1a4debc))
- **tools** Refuse a mistyped get_reference_trace option instead of defaulting it - ([4e5a002](https://github.com/noirbizarre/shaipe/commit/4e5a00234efd68ffc938abb5fb85dce7bc9da2ff))
- **tools** Stop write_variant claiming a usvg check it does not run - ([b014d29](https://github.com/noirbizarre/shaipe/commit/b014d2918efcca354451092d590c2e298fea7928))
- **tools** Report a failed --write save on stderr instead of dropping it - ([23b30a1](https://github.com/noirbizarre/shaipe/commit/23b30a1f3b7a72bb1e0e22b209c64c80655f01ef))
- **tools** Refuse an unsupported reference format in every reference tool - ([3179b5e](https://github.com/noirbizarre/shaipe/commit/3179b5e88aa32193e5990f836120ae7ee4e29408))
- **tools** Keep unsaved metadata edits when write_variant splices a variant - ([435c408](https://github.com/noirbizarre/shaipe/commit/435c4088db92cac67bb5c9d4d4735e794f08f45b))
- **tools** Get_svg serves the project as it stands, not as it was read - ([d2bd868](https://github.com/noirbizarre/shaipe/commit/d2bd868c3a4252af5cfaed0b10c2e08b8e24477a))
- **tui** Let ctrl-c stop a turn while a pane is being edited - ([316cc8c](https://github.com/noirbizarre/shaipe/commit/316cc8ceee1d2fe33e97a0881bbc21ffbeb5ae88))
- **tui** Export renders to the project's declared output directory - ([ea612e5](https://github.com/noirbizarre/shaipe/commit/ea612e5b8790cf887f63397064f666782a6f2ad1))
- **tui** Stop panes.rs's own status-line test from the same mtime flake - ([6bc83d3](https://github.com/noirbizarre/shaipe/commit/6bc83d3f80a169aae5d388367283799e36fc7e84))
- **tui** Stop three tests from depending on mtime resolution - ([f43077b](https://github.com/noirbizarre/shaipe/commit/f43077b7ff49dfe7a846951accee93450ceef185))
- **tui** Make the toolbar say what it does, and the arrows go somewhere - ([be3ee34](https://github.com/noirbizarre/shaipe/commit/be3ee346d053bbb47130ea313fc4ec21b1d3a68e))
- **tui** Put the key that reaches the agent back in the footer - ([4694487](https://github.com/noirbizarre/shaipe/commit/46944877552b9c56dc0ea20bf22ae38ef0372a78))
- **tui** A queued reload is no longer thrown away - ([1fdfb38](https://github.com/noirbizarre/shaipe/commit/1fdfb3839313a9e9a701e8311fb18df19357e653))
- **tui** Reach the agent from inside the prompt editor - ([7e280f2](https://github.com/noirbizarre/shaipe/commit/7e280f2e8803af4f0ad91de525391bfb682a5b11))
- **tui** Keep the editor on screen and say why an agent is missing - ([6c6411d](https://github.com/noirbizarre/shaipe/commit/6c6411d133ab7a3a40f6566932c8434bfaba1608))
- **vectorize** Derive the translucent line from the analysis's opaque floor - ([afcaf61](https://github.com/noirbizarre/shaipe/commit/afcaf61ab029323263b27c74580d423963296c35))
- **workflow** Ignore anti-aliasing fringes when counting a reference's colours - ([4b414fc](https://github.com/noirbizarre/shaipe/commit/4b414fc4c7965012068080b93ec08d4827f46794))

### ⚡ Performance

- **tui** Halve preview traffic under tmux, and show the spinner - ([9871f1a](https://github.com/noirbizarre/shaipe/commit/9871f1aacd83658b737247de47cfe947b71d6119))
- Stop rasterising far more pixels than anyone can see - ([b3b21b8](https://github.com/noirbizarre/shaipe/commit/b3b21b821be2076e62417c7134e4b97f28beef68))

### 🔨 Refactor

- **preview** Use ratatui-image instead of hand-written backends - ([6dc3af2](https://github.com/noirbizarre/shaipe/commit/6dc3af2bc6d100636e04657acc68ce6b8777fba1))
- **tools**  🚨 **breaking** Rename the tools to verb_noun - ([91ac627](https://github.com/noirbizarre/shaipe/commit/91ac6272b171fecc984b50ce00ce8f6cbb05c9a8))
- **tools**  🚨 **breaking** Describe tool inputs with JSON Schema - ([3132b83](https://github.com/noirbizarre/shaipe/commit/3132b8330066c00e1966eabee9e4ecd3247d5f6d))
- **tui** Drive the workspace from an async event loop - ([70a3135](https://github.com/noirbizarre/shaipe/commit/70a3135720993dce18509a2e9e88bf7fc4cc3b34))

### 📚 Documentation

- **adr** Wrap the index entries the full titles made too long - ([ba8b493](https://github.com/noirbizarre/shaipe/commit/ba8b493d4207732a7595f1081be39b70d577abcf))
- **adr** Align the ADR index and process with practice, and record drift as Updates - ([b9ac422](https://github.com/noirbizarre/shaipe/commit/b9ac4225474487f2d8a33c03d801969333baba8f))
- **adr** Record the text logo in ADR-005 and the new verbs in ADR-007 - ([f1502b6](https://github.com/noirbizarre/shaipe/commit/f1502b64d757a0e9f2c29526d5923875d1ec0d3e))
- **adr** Document the reference reconstruction architecture ([#25](https://github.com/noirbizarre/shaipe/issues/25)) - ([43a0deb](https://github.com/noirbizarre/shaipe/commit/43a0debe72b176dd8ddd07809494690bbc16e880))
- **cli** Document that --source/--inspiration resolve from cwd, not the project - ([b44ef8a](https://github.com/noirbizarre/shaipe/commit/b44ef8ad59855b3df6cde1e8e84c165c55b398bb))
- **logo** Use text instead of geometries - ([76baaae](https://github.com/noirbizarre/shaipe/commit/76baaae6b9695a5635be1d413bf8187ad4539a8c))
- **plan** Mark epic 1.5 and appearance analysis done - ([870593c](https://github.com/noirbizarre/shaipe/commit/870593c8db6d13d68cf8615be85c76834e0ab2d2))
- **plan** Strike through superseded TUI items and tick the finished scroll one - ([4e47216](https://github.com/noirbizarre/shaipe/commit/4e472163d6e8b2b83998b536168520c86e7e336f))
- **vectorize** Correct the trace docs that still said gradients are never produced - ([8f25309](https://github.com/noirbizarre/shaipe/commit/8f2530912a3aba07adfd14c87e757e0baa5c9825))
- Bring ADR-014/024/025, README and PLAN up to date with appearance work - ([bb8cbdd](https://github.com/noirbizarre/shaipe/commit/bb8cbddbace0d61001cb8c2f7d8516e8c35a75de))
- Add roadmap for visual understanding, UX, and harness integration - ([96c50bd](https://github.com/noirbizarre/shaipe/commit/96c50bd6073119371b923ac74dac5992a8a2e686))
- Correct stale comments and rustdoc that contradict the code - ([070c909](https://github.com/noirbizarre/shaipe/commit/070c909fa6e29459ec21cd48d8c8077f7250e53f))
- Shaipe names a model back but never chooses one; opencode.rs restricts, not the only OpenCode code - ([6df63e4](https://github.com/noirbizarre/shaipe/commit/6df63e414104b76cfab124e82304bd7abb95cd62))
- List every module and edge, and stop calling mise run ci the same as CI - ([451bb4c](https://github.com/noirbizarre/shaipe/commit/451bb4cff0a3144523edd1f5218cfd0169daa8ad))
- Install from source until the first release exists - ([65f3e57](https://github.com/noirbizarre/shaipe/commit/65f3e578630a9a76f6423a6a4b69bbe9db61cb79))
- Say where a pinned font is fetched, inside Renderer::new - ([ba04d9a](https://github.com/noirbizarre/shaipe/commit/ba04d9addffcc7040e9b2b6e8e93436f8582addf))
- Fix the git-tpl note and record ADR-005 and ADR-007 drift - ([7ee8eb8](https://github.com/noirbizarre/shaipe/commit/7ee8eb8d39c2ef52b0df5f0ebd4395994f1dcb0a))
- Correct the README's CLI, permission and feature claims - ([2ce2f6f](https://github.com/noirbizarre/shaipe/commit/2ce2f6f1a8765f79548751aadc77bd899db270fb))
- Fix markdown lint violations surfaced by the template's new lint:md task - ([e95146e](https://github.com/noirbizarre/shaipe/commit/e95146e965cdb3a8f408be198dfdaed784592e01))
- Say what the workspace and the agent can each actually do - ([32c36dc](https://github.com/noirbizarre/shaipe/commit/32c36dcd699ab7c542c799b9b62074a64cb3cd33))
- The prompt pane has two modes now - ([d5d2df2](https://github.com/noirbizarre/shaipe/commit/d5d2df2377ae512593015aa72c23229f08e7baef))
- Say what --yes actually grants - ([c56e730](https://github.com/noirbizarre/shaipe/commit/c56e730eb27017829cc4b201cf2eda817bd28718))
- Record the agent integration - ([8b4cdcc](https://github.com/noirbizarre/shaipe/commit/8b4cdcc2ba693f174dbe318b201d7d563bdb332f))

### 🧪 Tests

- **mcp** Make the stdout-purity test able to fail, and say why suppress matters - ([6440089](https://github.com/noirbizarre/shaipe/commit/6440089e5141b58a60315f0d945755c3e6d374fa))
- **reconstruction** Add appearance fixtures and evaluation coverage ([#46](https://github.com/noirbizarre/shaipe/issues/46)) - ([bea46f4](https://github.com/noirbizarre/shaipe/commit/bea46f4b8a800b23b60116c9484d0a7c6ad68f55))
- **reconstruction** Assert the traced SVG's structure, not its bytes - ([49cc82c](https://github.com/noirbizarre/shaipe/commit/49cc82cd6401ea16a0fc48407e2af78d198ada31))
- **reconstruction** Prove the reference loop end to end without a model - ([e0be4b7](https://github.com/noirbizarre/shaipe/commit/e0be4b785eee3e3321d7a2d9504a58dcce4df46f))
- **tools** Check every text that names a tool against the registry - ([1fce76a](https://github.com/noirbizarre/shaipe/commit/1fce76a4732ed077b9a101c09c1a2dc15bedd427))

### 🎨 Style

- **tests** Format the stdout-purity fixture helper - ([4028021](https://github.com/noirbizarre/shaipe/commit/40280216b6f4192e31891c41cb6f6093ab5b6968))

### 🧹 Chores

- **prek** Make the backend hook check what it claims - ([3c5f116](https://github.com/noirbizarre/shaipe/commit/3c5f116884eec763e73166293e3ae738679c3795))
- **tpl** Update to rust.tpl@26ef66c - ([4a06226](https://github.com/noirbizarre/shaipe/commit/4a06226bc40f0481cb2a43da50f8dac495ade037))
- Bootstrap from rust.tpl - ([3c1437b](https://github.com/noirbizarre/shaipe/commit/3c1437b75df72705d19c8e63efa426aaa45b878f))

### Plan

- Separate project description from agent transcript - ([52e1771](https://github.com/noirbizarre/shaipe/commit/52e17717e7afc8a7a8edd3b4ae7c45a2f1da1aec))

### Tpl

- Render rust at main - ([39cc324](https://github.com/noirbizarre/shaipe/commit/39cc32493ad2b57b9042681293bfe200e83d0e16))
- Render rust at main - ([77b34af](https://github.com/noirbizarre/shaipe/commit/77b34afff5be191c7ac6167f45a7427d418463e8))

## ❤️ New Contributors

* @noirbizarre made their first contribution
