"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		_.hr("cRxAtf");
		_.Lr = class {
			constructor() {
				this.Hsb = 317594;
				this.dialog = _.m(_.rC);
				this.S = _.Dk;
			}
			Mp() {
				this.dialog.open(_.MH);
			}
		};
		_.Lr.J = function(a) {
			return new (a || _.Lr)();
		};
		_.Lr.ka = _.u({
			type: _.Lr,
			da: [["ms-upgrade-options-callout"]],
			ha: 10,
			ia: 2,
			la: [
				[
					1,
					"quota-exceeded-container",
					"g1-gradient-border"
				],
				[1, "quota-exceeded-message"],
				[1, "quota-success-message"],
				[1, "quota-exceeded-body"],
				[1, "buttons-container"],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					3,
					"click",
					"ve",
					"veClick"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.I(0, "mat-divider"), _.F(1, "div", 0)(2, "div", 1)(3, "div", 2), _.R(4, "Upgrade to go further with Gemini"), _.H(), _.F(5, "div", 3), _.R(6, " You’ve reached your quota for the day, you can wait for it to reset, upgrade or link an API key to continue and unlock even higher limits. "), _.H()(), _.F(7, "div", 4)(8, "button", 5), _.J("click", function() {
						return b.Mp();
					}), _.R(9, " Continue to upgrade "), _.H()()();
				}
				if (a & 2) {
					_.y(8), _.E("ve", b.Hsb)("veClick", true);
				}
			},
			dependencies: [
				_.Yy,
				_.tz,
				_.OD,
				_.ND,
				_.Bz
			],
			styles: ["@property --quota-upgrade-angle{syntax:\"<angle>\";initial-value:175deg;inherits:false}.quota-exceeded-container[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-weight:400;line-height:18px;-webkit-backdrop-filter:blur(87px);backdrop-filter:blur(87px);background:transparent;border-radius:8px;border:none;color:var(--color-v3-text-on-button);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;font-size:12px;gap:8px;padding:12px;margin-block:24px 32px;position:relative}.quota-exceeded-container.g1-gradient-border[_ngcontent-%COMP%]{--conic-neutral-color:var(--color-v3-outline-var);--conic-blue-color:#4285f4;--conic-green-color:#1aa64a;--conic-yellow-color:#fcbd00;--conic-red-color:#db372d;--conic-red-neutral-mix:color-mix(in srgb,var(--conic-red-color) 64.5%,var(--conic-neutral-color));border-top:none}.quota-exceeded-container.g1-gradient-border[_ngcontent-%COMP%]:before{content:\"\";position:absolute;inset:0;border-radius:inherit;padding:1px;background:conic-gradient(from var(--quota-upgrade-angle,175deg) at 50% 50%,var(--conic-red-neutral-mix) 0deg,var(--conic-neutral-color) 69.68deg,var(--conic-blue-color) 115.09deg,var(--conic-green-color) 193.5deg,var(--conic-neutral-color) 269.8deg,var(--conic-yellow-color) 291.9deg,var(--conic-red-color) 322.17deg,var(--conic-red-neutral-mix) 1turn);-webkit-mask:-webkit-gradient(linear,left top,left bottom,color-stop(0,#fff)) content-box,-webkit-gradient(linear,left top,left bottom,color-stop(0,#fff));-webkit-mask:-webkit-linear-gradient(#fff 0 0) content-box,-webkit-linear-gradient(#fff 0 0);-webkit-mask-composite:xor;mask-composite:exclude;pointer-events:none}.quota-exceeded-container[_ngcontent-%COMP%]   [ms-button][_ngcontent-%COMP%]{width:-webkit-fit-content;width:-moz-fit-content;width:fit-content}.quota-exceeded-container[_ngcontent-%COMP%]   .quota-exceeded-message[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:4px}.quota-exceeded-container[_ngcontent-%COMP%]   .quota-success-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px}.quota-exceeded-container[_ngcontent-%COMP%]   .quota-exceeded-body[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var);max-width:470px}.quota-exceeded-container[_ngcontent-%COMP%]   .buttons-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:start;-webkit-justify-content:flex-start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:flex-start}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

