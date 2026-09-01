# Aster
**Another Lua IDE**

Aster is a small desktop IDE for Lua. You can edit and run code like normal, but it also keeps track of modules, multiple returns, common performance mistakes, and class systems  built out of tables and metatables

---
## Features
- Open an existing folder or make a new Lua project (I hope...)
- Problems and performance suggestions separated
- See how modules connect through `require`
- Inspect member lookups through common `__index` and `setmetatable` patterns
- Detect Lua 5.1, 5.3, 5.4, and LuaJIT 2.1 installs available on your `PATH`

---
## Getting it
Grab the latest build from [Releases](../../releases).

> **Windows only for now... again**

---
## First launch
Aster opens to your recent projects instead of throwing you straight into a random folder.

Open a folder, pick the Lua runtime you want, and that is pretty much it. If Aster can find the runtime on your `PATH`, you can run the current file with the Run button or `F5`.

---
## How to use this thing
### Problems and Suggestions
**Problems** are things that are PROBABLY wrong. Unknown members, bad return usage, unresolved modules, etc

**Suggestions** are code that may work perfectly fine but could be wasteful. They're kept separate for that reason

### Member Lookup
Put your cursor on a member access and open **Members**. It'll will show the lookup path it could prove from the source, including class tables, constructors, metatables, and `__index` chains.

It's mostly just static analysis, if the lookup depends on runtime values, dynamic functions or metatable tricks, Aster may tell you it does not know

### Module Graph
The graph view follows ordinary `require` calls and shows how the project fits together. Cycles and unresolved modules are called out

### Running code
Aster uses the Lua runtime selected in Settings. Output and errors show up in the bottom drawer, and you can stop a running process from there too

---
## Will it understand my 4,000-line homemade class framework?
Aster SHOULD understand the common patterns like class tables, constructors, table `__index`, `setmetatable`, inherited methods, and instance fields assigned through `self`. But ofc there's a bunch of factors that play a part and it might trip up. Please make an issue if that happens...

---
## Building it yourself
If you are into that:

- [Rust](https://rustup.rs/) (latest stable)
- Node.js and npm
- The normal [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform
- A Lua runtime if you want to run Lua from inside the IDE

```bash
git clone https://github.com/Izcahuatl/aster.git
cd aster
cargo test --workspace
cargo run -p aster-ide
```

For a release build:

```bash
npx -y @tauri-apps/cli@2 build
```

There is also a command-line analyzer if you only want the checking bits:

```bash
cargo run -p aster-cli -- --help
```

---
## Waaaahh it broke
**Aster cannot find Lua**
- Make sure Lua is installed.
- Make sure its executable is available on your `PATH`.
- Check the selected runtime in Settings.

**Member Lookup says it cannot resolve something**
- Make sure your cursor is actually on a member access.
- Run a workspace check so the project model is current.
- If the value is assembled dynamically at runtime, Aster genuinely may not be able to prove it.

**Run does nothing useful**
- Open the Output tab and read what Lua complained about.
- Make sure the active tab is a `.lua` file.
- Try the same file with your Lua executable in a terminal. Sometimes the code is simply exploding normally.

**The project looks out of date**
- Hit **Check** to analyze the workspace again.
- Refresh the Explorer if files changed outside Aster.

---
## Serious License Stuff
Copyright (C) 2026 Izcahuatl

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. It can also be found at <https://www.gnu.org/licenses/>.
