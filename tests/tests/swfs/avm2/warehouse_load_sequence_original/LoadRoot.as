package {
    import flash.display.MovieClip;
    import flash.display.Loader;
    import flash.events.Event;
    import flash.net.URLRequest;
    public class LoadRoot extends MovieClip {
        private var outer:MovieClip;
        private var loader:Loader;
        private var frames:uint = 0;
        private var ready:uint = 0;
        private var rounds:uint = 0;
        private var errors:uint = 0;
        private var first:uint;
        public function LoadRoot() {
            addEventListener(Event.ENTER_FRAME, step);
            begin();
        }
        private function begin():void {
            outer = new MovieClip();
            var inner:MovieClip = new MovieClip();
            outer.addChild(inner); addChild(outer);
            loader = new Loader(); inner.addChild(loader);
            loader.contentLoaderInfo.addEventListener(Event.COMPLETE, complete);
            loader.load(new URLRequest("asset.swf"));
        }
        private function complete(e:Event):void {
            ready = frames + 1;
            first = uint(Object(loader.content).runs);
            // Reparent after the loaded content's constructor registered scripts.
            removeChild(outer);
            var fresh:MovieClip = new MovieClip();
            fresh.addChild(outer); addChild(fresh); outer = fresh;
        }
        private function step(e:Event):void {
            frames++;
            if (ready > 0 && frames >= ready + 8) {
                if (uint(Object(loader.content).runs) <= first) { errors++; trace("FAIL load-script " + rounds); }
                ready = 0;
                rounds++;
                loader.unloadAndStop();
                removeChild(outer);
                if (rounds < 32) begin();
                else {
                    removeEventListener(Event.ENTER_FRAME, step);
                    if (errors == 0) trace("PASS load " + rounds);
                    else trace("FAIL load " + errors);
                }
            }
            if (frames == 1024 && rounds < 32) trace("FAIL load-timeout " + rounds);
        }
    }
}
