package {
    import flash.display.Loader;
    import flash.display.MovieClip;
    import flash.events.Event;
    import flash.net.URLLoader;
    import flash.net.URLRequest;
    import flash.system.ApplicationDomain;
    import flash.system.LoaderContext;

    public class Test extends MovieClip {
        private var request:URLLoader = new URLLoader();
        private var loaded:Loader = new Loader();

        public function Test() {
            request.dataFormat = "binary";
            request.addEventListener(Event.COMPLETE, onBytes);
            request.load(new URLRequest("loadable.swf"));
        }

        private function onBytes(event:Event):void {
            loaded.contentLoaderInfo.addEventListener(Event.INIT, onInit);
            loaded.contentLoaderInfo.addEventListener(Event.COMPLETE, onComplete);
            loaded.loadBytes(request.data, new LoaderContext(false, new ApplicationDomain()));
        }

        private function onInit(event:Event):void {
            trace("init marker", loaded.contentLoaderInfo.applicationDomain.hasDefinition("AfterImport"));
        }

        private function onComplete(event:Event):void {
            trace("complete marker", loaded.contentLoaderInfo.applicationDomain.hasDefinition("AfterImport"));
        }
    }
}
