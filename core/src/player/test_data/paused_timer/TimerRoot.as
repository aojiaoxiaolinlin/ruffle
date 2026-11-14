package {
    import flash.display.MovieClip;
    import flash.utils.setInterval;
    public class TimerRoot extends MovieClip {
        public function TimerRoot() { setInterval(onTimer, 10); }
        private function onTimer():void { trace("TIMER"); }
    }
}
