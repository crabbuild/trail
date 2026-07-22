# Changelog

All notable changes to Trail are documented in this file. Trail follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0](https://github.com/crabbuild/trail/compare/v0.1.0...v1.0.0) (2026-07-22)


### ⚠ BREAKING CHANGES

* **mcp:** expose lane merge queue tools
* **api:** expose lane merge queue routes
* **cli:** nest merge queue under lanes
* **trail:** make merge queue lane-only
* **trail:** unify lane merge interfaces
* **cli:** human output now uses the unified terminal renderer.\nUse --color never instead of --no-color, and use JSON or NDJSON\nfor automation.

### Features

* add changed-path ledger state store ([17bff71](https://github.com/crabbuild/trail/commit/17bff713b8a58a699f72b53207203192541e7da4))
* add durable changed-path observer log ([11a2913](https://github.com/crabbuild/trail/commit/11a2913c1524818e2026be82b971577aa6936ac0))
* add durable lane initialization schema ([9cca4fd](https://github.com/crabbuild/trail/commit/9cca4fd3b63dedaf7af328941bfb274ef1eae26d))
* add lane initialization owner schema ([f032443](https://github.com/crabbuild/trail/commit/f032443f0d4cf564fa7c261efb11eaa833182d8a))
* add qualified Linux change observer ([3a65b83](https://github.com/crabbuild/trail/commit/3a65b83e26f73a3419fce6bcb1ecaa77924437c3))
* add qualified macOS change observer ([e107e86](https://github.com/crabbuild/trail/commit/e107e86eb69a404e2d7bd394b2b50e0d8e9838b2))
* add stable git handoff errors ([a8c938f](https://github.com/crabbuild/trail/commit/a8c938f97c835c795a6e802c0aaf74d3ebc74115))
* add streamed changed-path reconciliation ([0788d1c](https://github.com/crabbuild/trail/commit/0788d1cd993706a6f6eadbfbcd5c593577b04bc8))
* add universal environments and agent capture ([5625066](https://github.com/crabbuild/trail/commit/5625066bd86cc45f1097a9383cd55d9f91225d6b))
* auto-start authenticated workspace daemon ([5aca1e4](https://github.com/crabbuild/trail/commit/5aca1e4c3a4d4c8215b2fadaf5e8ea0992197d5b))
* capture ACP client callbacks ([9211b55](https://github.com/crabbuild/trail/commit/9211b5567451c2b65085759c1a2e5a869b702e47))
* capture ACP session lifecycle ([00bf631](https://github.com/crabbuild/trail/commit/00bf631278953e847a4599ea81e8db996f6ef3fb))
* capture ACP turns and cancellation ([f382127](https://github.com/crabbuild/trail/commit/f382127f8dbd0c040515c1b53384cbb4c2da77db))
* capture every ACP session update ([502b98d](https://github.com/crabbuild/trail/commit/502b98d7928c867ffcae64957aeae6d940ab76b7))
* **cli:** cut over terminal UX ([8d619c4](https://github.com/crabbuild/trail/commit/8d619c461b6f56367617a929cc8cf0eeeac5b01f))
* consume fenced changed-path snapshots ([6a938d0](https://github.com/crabbuild/trail/commit/6a938d0693ac7268504f3092695e8c14ccaedd2f))
* coordinate lane initialization in sqlite ([7cb49cd](https://github.com/crabbuild/trail/commit/7cb49cd2f75ce08f304e85b4c6dbc363a0cb6ac9))
* cover workspace and lane producers with intents ([5ed9603](https://github.com/crabbuild/trail/commit/5ed9603326918720721e98de6f8accdf74559172))
* enforce schema v18 hard cutover ([9250041](https://github.com/crabbuild/trail/commit/92500415ac5db9de6dcaf482ff8cf0ec669d6354))
* enforce strict native cow materialization ([9b05618](https://github.com/crabbuild/trail/commit/9b05618c5d9a692dde9e4413d59529c4e1280fa4))
* hard cut over to fuse cow ([9a9278e](https://github.com/crabbuild/trail/commit/9a9278e969c8ea7bfc11be07235eb37691d471f7))
* harden native cow lifecycle reporting ([e78fcb0](https://github.com/crabbuild/trail/commit/e78fcb021ea469c0cb6d00acff68bde25babaca1))
* journal ACP capture off the relay path ([8c80444](https://github.com/crabbuild/trail/commit/8c80444711cfacf27cc9352965e18e63b956c80c))
* persist changed-path policy dependencies ([80f668a](https://github.com/crabbuild/trail/commit/80f668a3d601506c1ab7d4153a732c5a5fc95f1c))
* recover changed-path intents and lifecycle ([1dd5d01](https://github.com/crabbuild/trail/commit/1dd5d01704626f13079a207a882177e2ada08279))
* rename full cow to native cow ([5b50912](https://github.com/crabbuild/trail/commit/5b50912a89378aeb354349f58450d59745e4c196))
* replay concurrent lane initialization ([165d628](https://github.com/crabbuild/trail/commit/165d628ff5fb56767cf986dd1af38a3b18854996))
* report native COW lane space ([0366d4c](https://github.com/crabbuild/trail/commit/0366d4c31962ce0312392a6b62c539636b573227))
* reserve lane spawns idempotently ([0e8629c](https://github.com/crabbuild/trail/commit/0e8629cfb81f39c11284a81af549c4898106ca45))
* resume lane initialization across failures ([0eee7d5](https://github.com/crabbuild/trail/commit/0eee7d595c0e0cac0868b320b67d7113364efd2a))
* scale changed-path ledger for large repositories ([9e10f7c](https://github.com/crabbuild/trail/commit/9e10f7cb337661b23b369ad8ce63ffab97fc0013))
* split agent ACP and hooks setup ([5f7010d](https://github.com/crabbuild/trail/commit/5f7010d3f34bf0fc5068bd0e0406228d6fc93feb))
* **trail:** clarify identifier prefixes ([df3d286](https://github.com/crabbuild/trail/commit/df3d286158e0f131c5983c47f8b4d68c516d238c))
* **trail:** unify lane merge interfaces ([3ad0a60](https://github.com/crabbuild/trail/commit/3ad0a603662a927add1936a9f2e177ac78e510af))
* validate ACP v1 transformations ([a310544](https://github.com/crabbuild/trail/commit/a310544fda1e5b4126ef32d07a50c7e43469447f))


### Bug Fixes

* **acp:** keep editor writes in lanes ([9565ea1](https://github.com/crabbuild/trail/commit/9565ea1bae9b98d8dbd4cf532d774705afbf28c8))
* activate authenticated schema fanout on Linux ([7aa736f](https://github.com/crabbuild/trail/commit/7aa736f7b2173267c7fd2f2cbc87ef0b9e23df45))
* admit concurrent lane observers safely ([fbfc89c](https://github.com/crabbuild/trail/commit/fbfc89cc189cf6a71b975c58b3029a846b0bb5d5))
* admit materialized lane spawns before database open ([8048446](https://github.com/crabbuild/trail/commit/804844670b0da63d04c086032a0db4e301c5ed3c))
* align workspace view journal generations ([1dfcf43](https://github.com/crabbuild/trail/commit/1dfcf430e9df05ffb7bf82c8428a986a0870b7e9))
* authenticate intent sidecars and stage restores ([b3015d2](https://github.com/crabbuild/trail/commit/b3015d2e8de50cc84d7779d81cafaab534a8d1d5))
* avoid idle schema fanout delay ([f3d7a32](https://github.com/crabbuild/trail/commit/f3d7a32abd107543ab8ccd42ea51b6cf513a7f75))
* avoid local clone race in scale harness ([337fcff](https://github.com/crabbuild/trail/commit/337fcffe3a5e30d86ea68bcee8b07afc06cd7124))
* avoid nested open recovery self-lock ([0649dc2](https://github.com/crabbuild/trail/commit/0649dc24852d706062addc29584618eac780fec3))
* bind Linux observer proof to durable publication ([10a3efe](https://github.com/crabbuild/trail/commit/10a3efe8e4ace3a0ebf6a3a48018b750e9504672))
* bind policy selectors and unsafe paths ([2ff2e6d](https://github.com/crabbuild/trail/commit/2ff2e6deab82f966bab1981653b645aae01cde76))
* bind real scale harness evidence ([153ce86](https://github.com/crabbuild/trail/commit/153ce860314b705a94a66bea64b2648fb9d36399))
* bind reconciliation to observer continuity ([eeed30c](https://github.com/crabbuild/trail/commit/eeed30cd47afa5fdfb4e7ed642cfb7bc0fc82587))
* bind segment quarantine allocations atomically ([d13bab6](https://github.com/crabbuild/trail/commit/d13bab69a4408c88c2d9d71fb4b93c8be490d19b))
* bind validated schema handoffs across processes ([f7bd3fb](https://github.com/crabbuild/trail/commit/f7bd3fbf93417eff468066d6cef32e6ad3fd850e))
* bound ACP capture shutdown ([9fe2582](https://github.com/crabbuild/trail/commit/9fe2582ce9727c9936eef5ba4fc6219ce8fd4f34))
* bound supervisor registration ([760d256](https://github.com/crabbuild/trail/commit/760d256dec524b936a0ee659a6f04f503f388fe0))
* cap pre-intent daemon socket artifacts ([6f91386](https://github.com/crabbuild/trail/commit/6f9138645f576514faf4d66ab9e83840ade41c1f))
* clean up detached schema fanout ([6e59f9c](https://github.com/crabbuild/trail/commit/6e59f9c99b8e48f18502b942c0c914b6706d3b56))
* close daemon cleanup and transition races ([01508bf](https://github.com/crabbuild/trail/commit/01508bf88694d5d5036dbcd5671663d314677719))
* close daemon recovery review gaps ([419c89e](https://github.com/crabbuild/trail/commit/419c89e45ecfe98eb9854b334fbf28d5d74954cb))
* close deterministic daemon review gaps ([83c64ff](https://github.com/crabbuild/trail/commit/83c64ff3f24226ba5ce70c76cd98a33b34fb371b))
* close deterministic production test gaps ([00b0378](https://github.com/crabbuild/trail/commit/00b0378fee0a33c3612a6e62a7ad16025dfe2be7))
* close fenced snapshot review gaps ([49856ef](https://github.com/crabbuild/trail/commit/49856efa4a0c8ee18706b38a9978c80f899a2c0b))
* close lane contention review gaps ([693380b](https://github.com/crabbuild/trail/commit/693380ba3924f6dc6b12821e5fa985cfd2dec464))
* close lane initialization fence gaps ([9ba14dd](https://github.com/crabbuild/trail/commit/9ba14dd1b5b7d5d8bfc224e266b6452ab817f598))
* close observer log re-review gaps ([7da02b7](https://github.com/crabbuild/trail/commit/7da02b7baca49dd68aefc14862a1d4b2269e2ceb))
* close pre-merge correctness gaps ([4041f7f](https://github.com/crabbuild/trail/commit/4041f7f47976060c427f1e1c8e73b0f15577895e))
* close real-repo harness review gaps ([b29d5e3](https://github.com/crabbuild/trail/commit/b29d5e3f853343c8dfe1f792eb353c8173f255a6))
* close scale supervisor races ([9599665](https://github.com/crabbuild/trail/commit/9599665aec5d4add77e7c1424bb1dbcd100bf8de))
* coalesce overlapping open recovery ([68d9c27](https://github.com/crabbuild/trail/commit/68d9c2761bed9df414916fb399512dace839f885))
* compile command authority in release builds ([e9b4ba4](https://github.com/crabbuild/trail/commit/e9b4ba43919290302ed92dfbb37e9374c7006273))
* coordinate concurrent native lane materialization ([a496cb1](https://github.com/crabbuild/trail/commit/a496cb14c8469e4558da7c3fc8ce06402d4e6c34))
* coordinate observer record publication ([c24ed27](https://github.com/crabbuild/trail/commit/c24ed2750234483bcf9f7d45713b0912eec0f92a))
* count full root traversals in path metrics ([251ee1c](https://github.com/crabbuild/trail/commit/251ee1cccc56b1e44ca3917647cb6e6d1d2a7043))
* deduplicate Trail-owned MCP identity ([759cb52](https://github.com/crabbuild/trail/commit/759cb52a6bff7557ec7d88d50e34e3e3c78f6fa6))
* derive ACP capture outcome after drain ([7a79ca0](https://github.com/crabbuild/trail/commit/7a79ca047ee2e07bf8c2e1ada2b14c4b04c92eb3))
* detach schema validation fanout ([d799fe5](https://github.com/crabbuild/trail/commit/d799fe53f57ea7e1ffe1fe504cf753d347abc15a))
* drain ACP output after child exit ([780ef29](https://github.com/crabbuild/trail/commit/780ef29e0240dab1ec4716bc9c8031e3a21e01a0))
* exclude filtered paths from case fold index ([885a0a8](https://github.com/crabbuild/trail/commit/885a0a8ef5cbabbe7663f908c3b285a1b72f0356))
* fail closed on fallback process identities ([3545414](https://github.com/crabbuild/trail/commit/354541488ff7ceb12e302136fdd47c08e7ffa8d7))
* fail closed on policy dependency reuse ([8cd9a9f](https://github.com/crabbuild/trail/commit/8cd9a9f08bf6fc49dc33c2db03f6263854e5aa79))
* fence indeterminate lane owners ([d29acdb](https://github.com/crabbuild/trail/commit/d29acdb78d1c15725ba2c418b5ea1b8d445bbf30))
* fence lane initialization publications ([43ef530](https://github.com/crabbuild/trail/commit/43ef5300211971ab2f15103e8a43f2744ad74123))
* harden batched git plumbing ([6bcf94a](https://github.com/crabbuild/trail/commit/6bcf94a2f8d1a4f494a1e5cec5040205c19e26fe))
* harden changed-path ledger snapshots ([4d64c67](https://github.com/crabbuild/trail/commit/4d64c67494ead6a772a0d2db61473a4007c8e873))
* harden changed-path recovery durability ([92bd4bd](https://github.com/crabbuild/trail/commit/92bd4bd3d832082f0d2ad4128f57e04b8871fd8f))
* harden indexed record preflight ([80be733](https://github.com/crabbuild/trail/commit/80be7334805ac2762801ca58e782689901b7ca62))
* harden lane initialization authority ([ce04d57](https://github.com/crabbuild/trail/commit/ce04d57cbfcf6d0ece65d4a0594a16de0836c278))
* harden lane initialization backfill ([1e16da5](https://github.com/crabbuild/trail/commit/1e16da58bf74b1a3d904185f7da2db054dee944a))
* harden large-repository correctness invariants ([8e263a6](https://github.com/crabbuild/trail/commit/8e263a60ecc2b184c9a56bdbbccbe58fd310e0c5))
* harden legacy path index repair ([74ef18f](https://github.com/crabbuild/trail/commit/74ef18ffc71fd7f34895924bbf5a8dbc9379e7fe))
* harden macOS observer continuity ([34429ca](https://github.com/crabbuild/trail/commit/34429ca961a1aa1b39538c8c39776e91726f9c01))
* harden observer log crash protocol ([d90d256](https://github.com/crabbuild/trail/commit/d90d2569d9f60dbbcb99095f474b4040dd04325d))
* harden policy config and file reads ([a926b5a](https://github.com/crabbuild/trail/commit/a926b5ae52ef13300cb1385a4f5d951b7fb826d5))
* harden real-repo scale evidence ([24fb9c5](https://github.com/crabbuild/trail/commit/24fb9c5a7a724ca509f80b654a389192111e1fd1))
* harden schema handoff and segment retirement ([17b35a6](https://github.com/crabbuild/trail/commit/17b35a67cee7502a05e3c6dbf46b02ddedf845be))
* harden schema v18 immutable preflight ([223c387](https://github.com/crabbuild/trail/commit/223c387f27cfcccd1fc63e10b1e2195e558e0701))
* harden workspace daemon authority ([dec4629](https://github.com/crabbuild/trail/commit/dec4629f4783494f7e3034d1765aa2d93bd34884))
* honor configured environment temp root ([9ea3c87](https://github.com/crabbuild/trail/commit/9ea3c87e283e16f523c20abe1d554325d1389f43))
* ignore empty workspace setattr ([04cb4ec](https://github.com/crabbuild/trail/commit/04cb4ec70ec50f1c839f65c7f39ec9d2d46bf211))
* initialize submodules in layered CI ([9b2b494](https://github.com/crabbuild/trail/commit/9b2b494ff882e3d05c61bf113e9f713a65d23787))
* install compatible Dokany from MSI ([f9da272](https://github.com/crabbuild/trail/commit/f9da272784aa602fa0b338aeca8f9d6223232fa2))
* install Dokany for Windows releases ([e4d7c5a](https://github.com/crabbuild/trail/commit/e4d7c5ad1418dd3da90ef043dac1ae8cc0be2c72))
* isolate daemon-backed e2e state ([0b288f0](https://github.com/crabbuild/trail/commit/0b288f0462322e2018da352178963ff1c9bd2350))
* isolate reconciliation callback staging ([71cc37f](https://github.com/crabbuild/trail/commit/71cc37f77a8917524314a9b4152677bfcd7f3fb8))
* keep ACP capture shutdown bounded ([25447ac](https://github.com/crabbuild/trail/commit/25447aca17027dc7d96e4b064f6221cefb2658db))
* keep environment staging under Trail storage ([66b2991](https://github.com/crabbuild/trail/commit/66b29911c75d217cc1b84091a50e9fdf8b32a9e9))
* keep layered Windows CI actionable ([9ce84cc](https://github.com/crabbuild/trail/commit/9ce84cce078faca385575120c2780c51d3162033))
* keep release authority cfg clippy-clean ([83ac903](https://github.com/crabbuild/trail/commit/83ac90306696e9175a7aadd43d6b01bc6d4176f5))
* locate authenticated observer sidecars ([077f74f](https://github.com/crabbuild/trail/commit/077f74f46228d428ac35a87690f28cf8b334cec0))
* make ACP capture drain deterministic ([f65b046](https://github.com/crabbuild/trail/commit/f65b046e62f9c2ef2891526fd62a0032121e0482))
* make ACP checkpoint recording failure-safe ([a545fc1](https://github.com/crabbuild/trail/commit/a545fc1cbbd0a1c925e29a6d6bce7e5d4cf9292c))
* make concurrent observer retirement idempotent ([29bba84](https://github.com/crabbuild/trail/commit/29bba842727df1d74e3fb7de00fbc64b9ff0230c))
* make lane authority publication recoverable ([a45bf37](https://github.com/crabbuild/trail/commit/a45bf374c6e393e4ab2c03e755d240705ae33b92))
* make schema fanout lifecycle nonblocking ([aa3de6e](https://github.com/crabbuild/trail/commit/aa3de6e4caa55756f44c18bf3c41dfd228e97b67))
* make schema handoff parallel-safe ([b7d6d3f](https://github.com/crabbuild/trail/commit/b7d6d3f5cc8e1a085564614fb5f43fb4d55726f3))
* observe internal policy dependencies on Linux ([e25f998](https://github.com/crabbuild/trail/commit/e25f99801c0685af14e40ef6c8f6fd60a0c47476))
* order ACP capture finalization ([701b994](https://github.com/crabbuild/trail/commit/701b99467bfeb1be35883ebeea8190c771c3056a))
* parse ACP frames linearly ([daef66b](https://github.com/crabbuild/trail/commit/daef66b0c7aff55178f2c0a9eb42f714acce2e2f))
* persist ACP terminal ordering ([117c5b2](https://github.com/crabbuild/trail/commit/117c5b2884da6d68a673fb4dcd74c9404107415b))
* persist agent hook version ranges ([38a25c8](https://github.com/crabbuild/trail/commit/38a25c823eaf6b6bc6f740b24d13c11c3224c047))
* pin compatible Dokany runtime ([b2ee94b](https://github.com/crabbuild/trail/commit/b2ee94b5747bc906d80a91446976445ef42ff27e))
* pin Go adapter to the resolved toolchain ([a8b8cfd](https://github.com/crabbuild/trail/commit/a8b8cfdb5387a5a7b4a53d87075826699ad449b4))
* preserve ACP transport semantics ([1763181](https://github.com/crabbuild/trail/commit/1763181f2f9587c432b97584f93e117760aba6ba))
* preserve ACP workspace path semantics ([ceb916f](https://github.com/crabbuild/trail/commit/ceb916f2d9dbd0f46a0da61f5d530ef7d779a578))
* preserve concurrent lane handoff after daemon retirement ([0919f02](https://github.com/crabbuild/trail/commit/0919f02e1ebd552262dc39d998546a87186bf159))
* preserve dirty workspace journal tails ([96a25e8](https://github.com/crabbuild/trail/commit/96a25e86c6b03fc2997cb521b48cc1b3bdd525eb))
* preserve general index update ordering ([d3ae366](https://github.com/crabbuild/trail/commit/d3ae366d30781502a1199a163114d2bd16ff5ade))
* preserve ignored case aliases in manifests ([7d9dcfc](https://github.com/crabbuild/trail/commit/7d9dcfc4660b3ffb6984dc111846ca7dd2404fdf))
* preserve large-repository handoff correctness ([52e8e72](https://github.com/crabbuild/trail/commit/52e8e72ffd08bc4593d15375fb1feb11fac8bf9f))
* preserve live cow materializations ([751411f](https://github.com/crabbuild/trail/commit/751411f71e9ca6baec8bdfae7c32c129c6dfff1c))
* preserve pre-existing Git state in scale harness ([83825b3](https://github.com/crabbuild/trail/commit/83825b3c82115bf1548cf28ceff013b5fc181807))
* publish Homebrew after release workflow ([8aa4835](https://github.com/crabbuild/trail/commit/8aa483591e5d8a9433544d1a5469a589209f0a7c))
* qualify ignored native cow baselines ([69ae675](https://github.com/crabbuild/trail/commit/69ae675983cfc1075fe9653736afd4d124811ca7))
* qualify Trail under strict clippy ([9e478ec](https://github.com/crabbuild/trail/commit/9e478ec921e2353210b4d54821ec374d5b8787dc))
* recognize coalesced atomic rename evidence ([5b6ae6b](https://github.com/crabbuild/trail/commit/5b6ae6b56c4135ffacb6894aac633f66ef4d2e1a))
* recover orphaned daemon authority ([fa9933b](https://github.com/crabbuild/trail/commit/fa9933bc3cad069d62598889112a1d3a294f3f96))
* recover verified stale daemon handoff ([52390bc](https://github.com/crabbuild/trail/commit/52390bcc7420be1afe833b695ca3bcd14ee7a83c))
* remove broad lane initialization locks ([554c29f](https://github.com/crabbuild/trail/commit/554c29fe8ca9b7827e60d9d1f1a65c51d9ba2518))
* require clonefile-only scale copies ([96645e8](https://github.com/crabbuild/trail/commit/96645e89b64ec5f5bd804eeb5f292a00d914ec0d))
* require successful tool readiness probes ([b8056a8](https://github.com/crabbuild/trail/commit/b8056a883649748ac9008fe4e7c5fcc9cf7cf7f5))
* retire lane scopes before metadata transaction ([c8bde58](https://github.com/crabbuild/trail/commit/c8bde58f6cfe5150bb14e08c30c1ab26c1a77f79))
* retire workspace observer before materialized spawns ([27d3a0a](https://github.com/crabbuild/trail/commit/27d3a0a65683e55d518bf4e3ce4cdefeb5316367))
* retry authenticated lock generation turnover ([e573e8b](https://github.com/crabbuild/trail/commit/e573e8b236b6d80c7bc53c05af0c5ae5ff864013))
* retry concurrent schema handoffs ([221d473](https://github.com/crabbuild/trail/commit/221d473b45bc5c14e25dd364c927bbb284ceb55d))
* retry concurrent schema snapshot handoffs ([5acbb8e](https://github.com/crabbuild/trail/commit/5acbb8e0fe5d028155c24c1a8046824be82cada4))
* retry transient CLI schema handoffs ([f01f3f0](https://github.com/crabbuild/trail/commit/f01f3f07ed60b5da335c403757f61e6ab6cdc086))
* revalidate macOS observer authority ([0f4c08f](https://github.com/crabbuild/trail/commit/0f4c08f6bee8ec8669bf93094c59a8966f444cdc))
* revalidate observer lease at publication ([c64c0c2](https://github.com/crabbuild/trail/commit/c64c0c2327cc70e814e39fe5f41204c6e5d7324a))
* satisfy ACP v1 release gates ([de201e5](https://github.com/crabbuild/trail/commit/de201e58578c8939870b12e4def2ff7ac29164a2))
* satisfy strict adapter clippy ([3a79a4b](https://github.com/crabbuild/trail/commit/3a79a4b51274b8efda4fa20ba18ea5434a7bc329))
* seal policy trust and config discovery ([043e53f](https://github.com/crabbuild/trail/commit/043e53f929914a6b1aebcd2b142ade210ce04a3b))
* seal reconciliation publication ([64b5d63](https://github.com/crabbuild/trail/commit/64b5d635f655d2cdd3727d96530b88e7a2219c29))
* seal scale publication proofs ([14d3203](https://github.com/crabbuild/trail/commit/14d32035c92a1de5dd05714d81f8a13f046715ae))
* secure environment staging parent ([55c290c](https://github.com/crabbuild/trail/commit/55c290caf9af5c6ef941897166f6d75ba24920c7))
* serialize lane initialization repair ([4754385](https://github.com/crabbuild/trail/commit/47543853bf882b27218d2ba7dcc6c925a1c3b37b))
* serialize lock release and observer admission ([9bbadc6](https://github.com/crabbuild/trail/commit/9bbadc667b26693be0d3f49f626910a496b29e0a))
* serialize materialization database preflight ([69908a9](https://github.com/crabbuild/trail/commit/69908a9050c7f0c5292cf827c82b8d14259afba5))
* support current Prolly sqlite store API ([9fcd5ed](https://github.com/crabbuild/trail/commit/9fcd5ed868f2f6e486dc46d3f975ed9cfc635dd3))
* synchronize release lockfile ([5c44018](https://github.com/crabbuild/trail/commit/5c44018fc8b7f20d660e54ee822d69a7279cbdcf))
* tolerate concurrent volatile shm transitions ([52ef89a](https://github.com/crabbuild/trail/commit/52ef89aa815b921eca1ec1dcc1296f89c39ccdf0))
* **trail:** restore green output contracts ([bb3888d](https://github.com/crabbuild/trail/commit/bb3888d15878e2eff8df45049d597b3d4ccf7f72))
* update schema v20 user contracts ([732b2d0](https://github.com/crabbuild/trail/commit/732b2d002f658f815fc090eadb6bf45cb22c638b))
* use stable Darwin process identities ([56bea42](https://github.com/crabbuild/trail/commit/56bea42b2ddfc5f6801970c8bf81add6ccda184c))
* validate immutable ledger semantics ([b435ff1](https://github.com/crabbuild/trail/commit/b435ff1a0ca01e5e26cc59b48e13ac1ebc080911))
* wait for daemon retirement handoff ([3689cee](https://github.com/crabbuild/trail/commit/3689ceea22486c601e0ffa6a5c0a5d3d2057acc4))
* wait for Dokany MSI installation ([6f1682d](https://github.com/crabbuild/trail/commit/6f1682dc80cd7de5a93eb84032f6a3409112f821))


### Performance Improvements

* add full root path invariant index ([30e9bcc](https://github.com/crabbuild/trail/commit/30e9bcca830274226584e027f44de91430f0cef9))
* add indexed path invariant updates ([616ca4f](https://github.com/crabbuild/trail/commit/616ca4f30c30bd92248376107520bceff346dd2e))
* add opt-in operation metrics substrate ([95c92f0](https://github.com/crabbuild/trail/commit/95c92f0ee6668b53f302f7f93a6778702741de94))
* avoid reopening visible manifest paths ([c70119d](https://github.com/crabbuild/trail/commit/c70119d1db6899e3e35f575636be4262dd829014))
* batch canonical selected root reads ([1562e9f](https://github.com/crabbuild/trail/commit/1562e9fad90ebf2a5208e4e28ebe7ef7ed57e5b2))
* batch mapped git plumbing ([c345dbd](https://github.com/crabbuild/trail/commit/c345dbd99b520b49e10798d5f4faa55bcb0c6039))
* bound native COW lane parallelism ([f4616b2](https://github.com/crabbuild/trail/commit/f4616b240fe35b8504742b2a562cc6953253ca53))
* gate high-level git apply at scale ([6f9e4ba](https://github.com/crabbuild/trail/commit/6f9e4bafc0f56047b6cb9a96b62adab31ffe8583))
* gate path-index structure at scale ([b16ffb4](https://github.com/crabbuild/trail/commit/b16ffb4c880e29c36604aa477e4618773e211e6e))
* index selected worktree cache updates ([cd31cb1](https://github.com/crabbuild/trail/commit/cd31cb1065832387f3c907396de10c561e42c80b))
* linearize selected record construction ([97774ca](https://github.com/crabbuild/trail/commit/97774ca537d90a7ee4b298ad9d496efc4ac2f642))
* repair legacy path invariant indexes ([ea3ed39](https://github.com/crabbuild/trail/commit/ea3ed39527db32d8756f584c12e534243b002488))
* require mapped delta git export ([17f3ce0](https://github.com/crabbuild/trail/commit/17f3ce0cd9028935615f9c7526405739ef2edea8))
* reuse git state during agent apply ([5ad24c4](https://github.com/crabbuild/trail/commit/5ad24c4d9c8066e93eabbd597b3f9bdb402b362b))
* reuse selected operation policy ([3616dea](https://github.com/crabbuild/trail/commit/3616deac4bb72df5d28a749c88117aa35ec64871))
* use indexed path validation in hot mutations ([17bb9f8](https://github.com/crabbuild/trail/commit/17bb9f8d89bc079bebbbffcb890397bf4773a75d))


### Code Refactoring

* **api:** expose lane merge queue routes ([9eb6f34](https://github.com/crabbuild/trail/commit/9eb6f3403af925148010fafec7f44bf642314e83))
* **cli:** nest merge queue under lanes ([0579da4](https://github.com/crabbuild/trail/commit/0579da449497f66b13cb4607f5cae18e38217bff))
* **mcp:** expose lane merge queue tools ([9ea7d9a](https://github.com/crabbuild/trail/commit/9ea7d9a9f71d1e498317a0bc9c53cea3f30493c0))
* **trail:** make merge queue lane-only ([a85d93b](https://github.com/crabbuild/trail/commit/a85d93bcd6eaa6d8bc89d65e776c4888d5f4d83d))

## [Unreleased]

### Changed

- **Breaking:** Trail CLI human output now uses the unified outcome-first
  terminal renderer. The old human layouts and `--no-color` option are removed;
  use `--color never` instead.
- **Breaking:** `trail merge-lane` is removed. Use
  `trail lane merge <lane> --into <branch>` for lane-specific merges; the
  `trail merge` command remains for generic branch/ref merges.
- **Breaking:** `POST /v1/branches/{branch}/merge-lane` is removed. Use
  `POST /v1/lanes/{lane}/merge` with the target branch in the required `into`
  JSON field.
- **Breaking:** the generic merge queue is now lane-only. Use
  `trail lane merge-queue`, `/v1/lanes/merges/queue`, and
  `trail.lane_merge_queue_*`; the previous CLI, HTTP, MCP, resource, and
  `merge_queue` storage contracts are removed without aliases. Generic
  branches and refs continue through `trail merge`.
- Added `--format human|plain|json|ndjson`, `--color auto|always|never`, and
  `--pager auto|always|never`. `plain` is deterministic text; JSON and NDJSON
  are the supported contracts for automation.
- Status, diff, history, lane, agent, maintenance, and diagnostic output now
  use responsive tables, ordered checklists, explicit notices, and safe next
  actions. Human output is intentionally not stable for parsing.

## [0.1.0] - 2026-07-10

### Added

- Local-first operation history, branches, line provenance, and worktree recording.
- Isolated agent lanes with sessions, turns, patches, approvals, gates, and handoffs.
- Conflict-aware lane merges, merge queues, readiness reports, and recovery checkpoints.
- CLI, HTTP daemon, MCP stdio server, ACP relay, and Rust API integration surfaces.
- Backup, restore, filesystem checks, index rebuilding, and maintenance commands.

[Unreleased]: https://github.com/crabbuild/trail/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/crabbuild/trail/releases/tag/v0.1.0
