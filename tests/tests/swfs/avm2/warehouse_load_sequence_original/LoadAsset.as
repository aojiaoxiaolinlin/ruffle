package {
    import flash.display.MovieClip;
    public class LoadAsset extends MovieClip {
        public var runs:uint = 0;
        public function LoadAsset() { addFrameScript(0, record, 1, record); play(); }
        private function record():void { runs++; }
    }
}
