package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.display.SimpleButton;
    import flash.events.Event;
    public class SequenceRoot extends MovieClip {
        private var leaves:Array = [];
        private var chains:Array = [];
        private var buttons:Array = [];
        private var last:Array = [];
        private var frame:uint = 0;
        private var checks:uint = 0;
        private var errors:uint = 0;
        public function SequenceRoot() {
            for (var i:uint = 0; i < 128; i++) {
                var outer:MovieClip = new MovieClip();
                var inner:MovieClip = new MovieClip();
                var leaf:SequenceClip = new SequenceClip();
                outer.addChild(inner);
                if (i % 4 == 0) leaf.install();
                inner.addChild(leaf);
                if (i % 4 == 1) leaf.install();
                var button:SimpleButton = null;
                if (i % 8 == 7) {
                    button = new SimpleButton(new Sprite(), outer, new Sprite(), new Sprite());
                    addChild(button);
                    buttons.push(button);
                } else {
                    addChild(outer);
                }
                if (i % 4 == 2) leaf.install();
                if (i % 4 == 3) {
                    leaf.install(); leaf.clear(); leaf.install();
                }
                leaf.once = i % 8 == 3;
                leaves.push(leaf); chains.push(outer); last.push(0);
            }
            addEventListener(Event.ENTER_FRAME, advance);
        }
        private function advance(e:Event):void {
            frame++;
            if (frame % 8 == 0) {
                for (var i:uint = 0; i < leaves.length; i++) {
                    var leaf:SequenceClip = leaves[i];
                    if (leaf.runs <= uint(last[i])) {
                        errors++;
                        trace("FAIL silent " + frame + " " + i);
                    }
                    if (leaf.once && leaf.currentFrame != 2) {
                        errors++;
                        trace("FAIL queued-goto " + frame + " " + i);
                    }
                    checks++;
                    last[i] = leaf.runs;
                }
                for (i = 0; i < leaves.length; i++) {
                    leaf = leaves[i];
                    var old:MovieClip = chains[i];
                    if (i % 8 == 7) {
                        // Move a scripted subtree among inactive button states.
                        var b:SimpleButton = buttons[uint(i / 8)];
                        b.overState = new Sprite();
                        b.downState = old;
                        b.upState = new Sprite();
                    } else {
                        // Reuse a previously marked subtree under fresh ancestors.
                        if (old.parent) old.parent.removeChild(old);
                        var fresh:MovieClip = new MovieClip();
                        fresh.addChild(old);
                        addChild(fresh);
                        chains[i] = fresh;
                        if (frame % 32 == 0 && fresh.parent) fresh.parent.removeChild(fresh);
                    }
                    leaf.clear();
                    leaf.edited = false;
                    leaf.install();
                }
            }
            if (frame == 256) {
                removeEventListener(Event.ENTER_FRAME, advance);
                if (errors == 0) trace("PASS sequence " + checks);
                else trace("FAIL sequence " + errors);
            }
        }
    }
}
