"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		_.hr("Y0WYx");
		_.Jr = class {
			constructor() {
				this.Gsb = 317593;
				this.dialog = _.m(_.rC);
				this.Vb = _.m(_.AG);
				this.A = _.m(_.LH);
				this.yf = this.Vb.yf;
			}
			Mp() {
				if (this.Vb.Hb()) {
					this.A.openDialog();
				} else {
					this.dialog.open(_.MH);
				}
			}
		};
		_.Jr.J = function(a) {
			return new (a || _.Jr)();
		};
		_.Jr.ka = _.u({
			type: _.Jr,
			da: [["ms-navbar-upgrade-card"]],
			ha: 7,
			ia: 2,
			la: [
				[1, "upgrade-card-wrapper"],
				[
					1,
					"upgrade-card",
					3,
					"click",
					"ve",
					"veClick"
				],
				[1, "text-container"],
				[1, "title"],
				[1, "subtitle"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "button", 1), _.J("click", function() {
						return b.Mp();
					}), _.F(2, "div", 2)(3, "span", 3), _.R(4, "Upgrade to unlock more"), _.H(), _.F(5, "span", 4), _.R(6, "Access higher limits, Pro models, and more."), _.H()()()();
				}
				if (a & 2) {
					_.y(), _.E("ve", b.Gsb)("veClick", true);
				}
			},
			dependencies: [_.Bz],
			styles: ["[_nghost-%COMP%]{display:block;margin-bottom:12px;padding:0 8px}@property --upgrade-card-angle{syntax:\"<angle>\";initial-value:0deg;inherits:false}@property --conic-blue-color{syntax:\"<color>\";initial-value:rgba(66,133,244,.55);inherits:false}@property --conic-green-color{syntax:\"<color>\";initial-value:rgba(26,166,74,.5);inherits:false}@property --conic-yellow-color{syntax:\"<color>\";initial-value:rgba(252,189,0,.5);inherits:false}@property --conic-red-color{syntax:\"<color>\";initial-value:rgba(219,55,45,.5);inherits:false}.upgrade-card-wrapper[_ngcontent-%COMP%]{--conic-blue-color:rgba(66,133,244,.55);--conic-green-color:rgba(26,166,74,.5);--conic-yellow-color:rgba(252,189,0,.5);--conic-red-color:rgba(219,55,45,.5);position:relative;padding:1px;border-radius:16px;background:conic-gradient(from var(--upgrade-card-angle) at 50% 50%,var(--color-v3-outline-var) 0deg,var(--color-v3-outline-var) 69deg,var(--conic-blue-color) 115deg,var(--conic-green-color) 193deg,var(--color-v3-outline-var) 270deg,var(--conic-yellow-color) 291deg,var(--conic-red-color) 322deg,var(--color-v3-outline-var) 1turn);-webkit-animation:_ngcontent-%COMP%_rotate-upgrade-card-border 40s linear infinite;animation:_ngcontent-%COMP%_rotate-upgrade-card-border 40s linear infinite;box-shadow:0 3px 3px -1.5px rgba(10,13,18,.04),0 8px 8px -4px rgba(10,13,18,.03),0 20px 24px -4px rgba(10,13,18,.08);transition:--conic-blue-color .6s ease-in-out,--conic-green-color .6s ease-in-out,--conic-yellow-color .6s ease-in-out,--conic-red-color .6s ease-in-out}.upgrade-card-wrapper[_ngcontent-%COMP%]:after{content:\"\";position:absolute;inset:0;z-index:-1;border-radius:inherit;background:inherit;-webkit-filter:blur(14px);filter:blur(14px);opacity:.4;pointer-events:none;-webkit-transition:opacity .6s ease-in-out,-webkit-filter .6s ease-in-out;transition:opacity .6s ease-in-out,-webkit-filter .6s ease-in-out;transition:filter .6s ease-in-out,opacity .6s ease-in-out;transition:filter .6s ease-in-out,opacity .6s ease-in-out,-webkit-filter .6s ease-in-out}.upgrade-card-wrapper[_ngcontent-%COMP%]:hover{--conic-blue-color:#4285f4;--conic-green-color:#1aa64a;--conic-yellow-color:#fcbd00;--conic-red-color:#db372d;-webkit-animation-duration:8s;animation-duration:8s}.upgrade-card-wrapper[_ngcontent-%COMP%]:hover:after{-webkit-filter:blur(18px);filter:blur(18px);opacity:.7}@-webkit-keyframes _ngcontent-%COMP%_rotate-upgrade-card-border{to{--upgrade-card-angle:360deg}}@keyframes _ngcontent-%COMP%_rotate-upgrade-card-border{to{--upgrade-card-angle:360deg}}.upgrade-card[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container);border-radius:15px;border:none;-moz-box-sizing:border-box;box-sizing:border-box;cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:6px;padding:12px;text-align:start;-webkit-transition:background-color .2s ease-in-out;transition:background-color .2s ease-in-out;width:100%}.upgrade-card[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-surface-container-high)}.text-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;min-width:0}.title[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;font-weight:500;color:var(--color-v3-text)}.subtitle[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var)}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

