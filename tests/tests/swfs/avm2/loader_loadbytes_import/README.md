# loadBytes with an asynchronous import

`Test` downloads `loadable.swf` as bytes and passes it to `Loader.loadBytes`.
The loaded movie contains an `ImportAssets2` tag pointing to `empty.swf`, with
zero imported symbols. Its `AfterImport` class is defined **after** that tag.
Both INIT and COMPLETE must observe this class: processing the import must not
leave the remainder of the loaded movie unparsed.

The fixtures are original, minimal SWFs, without game assets. The expected
output expresses this regression invariant; it has not been captured from an
Adobe Flash Player run. The old Ruffle implementation produces neither event;
the fixed implementation produces both events with the class available.

To rebuild the fixtures with the ASC compiler and a playerglobal import ABC:

```sh
python3 build.py --asc /path/to/asc.jar --playerglobal /path/to/playerglobal_import.abc
```

The imported movie deliberately exports nothing. This reproduces the preload
pause independently of AVM2 symbol import support.
