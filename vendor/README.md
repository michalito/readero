# Pinned dependencies

`papers-document` and `papers-view` are the unmodified Rust bindings (including
their `sys` crates) from GNOME Papers 50.2, commit
`785ca168a98945f59d62308d664888c503ecf8ac`. Only their workspace dependency paths
are supplied by the Readero root manifest. Native libraries remain system
dependencies. See `PAPERS-COPYING` and the binding package license declarations.

`assets/foliate` contains the EPUB, CFI and paginator modules from foliate-js,
commit `78914aef4466eb960965702401634c2cb348e9b1`, with its MIT license.
Readero's own continuous controller lives outside that directory.

Update each group deliberately and run the renderer qualification suite;
neither upstream's crate version alone nor its development branch is a pin.
