# daedalus-sheet: third-party notices

This crate builds on, and from the sheet-metal work on adapts code from, two permissively licensed projects. Daedalus
itself is MPL-2.0; these licences allow that, with these notices kept.

- **SolidCraft** (https://github.com/tangency-labs/solidcraft, commit 871014f), Copyright (c) 2026 SolidCraft
  contributors, licensed under the MIT License or the Apache License, Version 2.0, at your option:
  `licences/SolidCraft-LICENSE-MIT`, `licences/SolidCraft-LICENSE-APACHE`. Its kernel crate (`solidcraft-kernel` and
  the crates it uses) is a dependency. Files adapted from SolidCraft's sheet-metal code name the file they come from at
  their top and say what was changed.
- **solidcore** (https://github.com/tangency-labs/solidcore), Copyright (c) 2026 The solidcore contributors, licensed
  under the Apache License, Version 2.0: `licences/solidcore-LICENSE`. It is a Rust port of NVIDIA's SMLib and SM_API
  (USD BRep); its notice, with NVIDIA's, is in `licences/solidcore-NOTICE`.
