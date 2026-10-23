package {
    import flash.display.Sprite;
    import flash.display.Loader;
    import flash.events.Event;
    import flash.utils.ByteArray;

    public class Test extends Sprite {
        private var loader:Loader = new Loader();
        private var phase:int = 0;
        private var unloads:int = 0;
        private var removals:int = 0;

        public function Test() {
            addChild(loader);
            loader.contentLoaderInfo.addEventListener(Event.COMPLETE, loaded);
            loader.contentLoaderInfo.addEventListener(Event.UNLOAD, unloaded);
            loadChild();
        }

        private function loadChild():void {
            // Empty AS3 SWF, embedded so the test also runs with mobile file sandboxes.
            var data:Array = [
                0x46, 0x57, 0x53, 0x0b, 24, 0, 0, 0,
                0x08, 0, 0, 0x18, 1, 0,
                0x44, 0x11, 8, 0, 0, 0,
                0x40, 0, 0, 0
            ];
            var bytes:ByteArray = new ByteArray();
            for each (var value:int in data) {
                bytes.writeByte(value);
            }
            bytes.position = 0;
            loader.loadBytes(bytes);
        }

        private function loaded(event:Event):void {
            unloads = 0;
            removals = 0;
            loader.content.addEventListener(Event.REMOVED_FROM_STAGE, removed);
            trace("phase " + phase);
            loader.unload();
            trace("unload events: " + unloads);
            trace("removedFromStage events: " + removals);
            trace("content is null: " + (loader.content == null));
            trace("children: " + loader.numChildren);
            loader.unload();
            trace("empty unload adds no events: " + (unloads == 1 && removals == 1));
            phase++;
            if (phase < 3) {
                loadChild();
            }
        }

        private function unloaded(event:Event):void {
            unloads++;
            // Bound recursion so an unfixed engine fails the test without crashing.
            if (phase == 0 && unloads < 3) {
                loader.unload();
            }
        }

        private function removed(event:Event):void {
            removals++;
            if (phase == 1 && removals < 3) {
                loader.unload();
            }
        }
    }
}
