package {
    import flash.display.MovieClip;
    public class SequenceClip extends MovieClip {
        public var runs:uint = 0;
        public var once:Boolean = false;
        public var edited:Boolean = false;
        public function SequenceClip() { play(); }
        public function install():void {
            addFrameScript(0, record, 1, record);
            play();
        }
        public function clear():void { addFrameScript(0, null, 1, null); }
        private function record():void {
            runs++;
            if (once && !edited) {
                edited = true;
                clear();
                // Queue a goto from a running script after removing both scripts.
                gotoAndStop(2);
            }
        }
    }
}
