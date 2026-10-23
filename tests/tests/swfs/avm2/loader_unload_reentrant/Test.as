package {
    import flash.display.Sprite;
    import flash.display.Loader;
    import flash.events.Event;
    import flash.net.URLRequest;

    public class Test extends Sprite {
        private var loader:Loader = new Loader();
        private var phase:int = 0;
        private var unloads:int = 0;
        private var removals:int = 0;

        public function Test() {
            addChild(loader);
            loader.contentLoaderInfo.addEventListener(Event.COMPLETE, loaded);
            loader.contentLoaderInfo.addEventListener(Event.UNLOAD, unloaded);
            loader.load(new URLRequest("child.swf"));
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
                loader.load(new URLRequest("child.swf"));
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
