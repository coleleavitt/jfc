"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	var window = this;
	try {
		_.hr("ommE1b");
		;
		_.AG.prototype.rqa = _.ca(173, function() {
			if (this.yf()) {
				var a = this.A;
				var b = !this.Le();
				a.A.set(b);
			}
		});
		var W0b = new _.he("AccountSwitcherConfigToken");
		var X0b = new _.he("AccountSwitcherWindowToken", { factory: () => window });
		var Y0b = [new _.oib()];
		var Z0b = function(a) {
			_.Bu(a.ti);
			if (a = a.r_a()) {
				a.nativeElement.focus();
			}
		};
		var $0b = function(a) {
			_.Af(document.body, "click").pipe(_.Qg()).subscribe((b) => {
				b = b.target;
				if (b.closest("#account-switcher") || b.closest("#account-switcher-button")) {
					$0b(a);
				} else {
					a.Ep.set(false);
				}
			});
		};
		_.Kr = class {
			constructor() {
				this.S = _.Dk;
				this.ve = {
					ifb: 317577,
					jfb: 317578,
					pfb: 317584,
					kfb: 317579,
					lfb: 317580,
					ofb: 317583,
					nfb: 317582,
					mfb: 317581
				};
				this.cta = "Sign in";
				this.dta = "Sign out";
				this.eta = "Switch account";
				this.G_ = Y0b;
				this.window = _.m(X0b);
				this.aa = _.m(_.WK);
				this.A = _.m(_.SK);
				this.ti = _.m(_.Hu);
				this.I = _.m(_.Op);
				this.X = _.m(W0b, { optional: true });
				this.U = _.m(_.OC);
				this.Vb = _.m(_.AG);
				this.Ilb = _.m(_.LH);
				this.H = _.m(_.pG);
				this.rb = _.m(_.Qp);
				this.Wfa = _.Ni("buttonContainer");
				this.r_a = _.Ni("firstFocus");
				this.R = this.I.getFlag(_.MKa);
				this.mh = this.Vb.mh;
				this.yf = this.Vb.yf;
				this.bb = this.U.bb;
				this.Le = this.Vb.Le;
				this.Bu = this.Vb.Bu;
				this.xla = _.W(() => !!this.rb.interactionId());
				this.NVa = _.W(() => this.xla() ? "Access method cannot be changed during an active session" : "");
				this.Hb = _.W(() => _.Nn(this.email()));
				this.z$a = _.W(() => this.yf() ? this.mh() === 3 ? this.R : true : false);
				this.jKb = _.W(() => this.yf() ? "Manage membership" : "Upgrade to Google AI");
				this.WQb = _.W(() => {
					var b = this.Hb();
					var c = this.yf() && this.mh() === 2;
					return !b && !c;
				});
				this.F = _.Zi(Object.assign({}, {}, { Xc: () => _.pf(_.JEb(this.aa)) }));
				this.NIb = _.W(() => {
					var b;
					var c;
					return ((b = this.F.error()) == null ? undefined : (c = b.error) == null ? undefined : c.status) === "UNAUTHENTICATED" ? false : true;
				});
				this.name = _.W(() => {
					var b;
					var c;
					var d;
					return (d = (b = this.F.value()) == null ? undefined : (c = b[0]) == null ? undefined : c.names[0]) != null ? d : "";
				});
				this.email = _.W(() => {
					var b;
					var c;
					var d;
					return (d = (b = this.F.value()) == null ? undefined : (c = b[0]) == null ? undefined : c.emailAddresses[0]) != null ? d : "";
				});
				this.photoUrl = _.W(() => {
					var b;
					var c;
					var d = (b = this.F.value()) == null ? undefined : (c = b[0]) == null ? undefined : c.photoUrls[0];
					return d ? new _.Cq(d, false).lG(64).LV(true).build() : "";
				});
				this.Ep = _.M(false);
				this.pIb = _.W(() => {
					var b = this.H.url();
					return _.qp(b);
				});
				this.TJb = _.W(() => {
					var b = Number.isInteger(_.yEb(this.A)) ? _.zEb(this.A) : "";
					b = b ? `/u/${b}` : "";
					var c = this.H.url();
					c = c.startsWith("/apps") || c.startsWith("/build");
					var d = this.mh() === 1;
					return `https://one.google.com${b}/ai?utm_source=ai_studio&utm_campaign=${c ? d ? "ais_build_limited_reached" : "ais_build_limit_reached" : d ? "ais_pg_limited_reached" : "ais_pg_limit_reached"}&utm_medium=web`;
				});
				_.W(() => this.mh() === 2);
				var a = Number.isInteger(_.yEb(this.A)) ? `?authuser=${_.zEb(this.A)}` : "";
				this.sHa = `https://policies.google.com/privacy${a}`;
				this.MLa = `https://myaccount.google.com/termsofservice${a}`;
			}
			z7() {
				return "Google Account: " + this.name() + " (" + this.email() + ")";
			}
			signOut() {
				var a = _.BEb(this.A);
				var b;
				var c = (b = this.X) == null ? undefined : b.jab[a];
				if (!c) {
					c = this.window.location.href;
				}
				a = _.On(new _.hk("https://accounts.google.com/Logout"), "continue", c);
				_.wd(this.window.location, _.kd(a.toString()));
			}
			signIn() {
				var a = _.On(new _.hk("https://accounts.google.com/ServiceLogin"), "continue", this.window.location.href);
				_.wd(this.window.location, _.kd(a.toString()));
			}
			PKa() {
				var a = _.On(_.On(new _.hk("https://accounts.google.com/AccountChooser"), "continue", this.window.location.href), "faa", "1");
				_.wd(this.window.location, _.kd(a.toString()));
			}
			rqa() {
				if (!this.xla()) {
					this.Vb.rqa();
				}
			}
			nO() {
				this.Ep.update((a) => !a);
				if (this.Ep()) {
					Z0b(this), $0b(this);
				}
			}
			V$(a) {
				a.stopPropagation();
				this.Ep.set(true);
				Z0b(this);
				$0b(this);
			}
			r5(a) {
				a.stopPropagation();
				this.Ep.set(false);
				this.Wfa().nativeElement.focus();
			}
		};
		_.Kr.J = function(a) {
			return new (a || _.Kr)();
		};
		_.Kr.ka = _.u({
			type: _.Kr,
			da: [["ms-account-switcher"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.Wfa, U0b, 5)(b.r_a, V0b, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			ha: 4,
			ia: 1,
			la: () => [
				["avatar", ""],
				[
					"buttonContainer",
					"",
					"trigger",
					"cdkOverlayOrigin"
				],
				["firstFocus", ""],
				"Privacy Policy",
				"Terms of Service",
				" �0� ",
				[
					"id",
					"account-switcher-button",
					1,
					"container"
				],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					1,
					"signin-button",
					3,
					"ve",
					"veClick"
				],
				[
					"role",
					"button",
					"tabindex",
					"0",
					"cdkOverlayOrigin",
					"",
					1,
					"button-container",
					3,
					"click",
					"keydown.enter",
					"keydown.space",
					"keydown.escape",
					"matTooltip",
					"ve",
					"veClick"
				],
				[1, "avatar-ring-wrapper"],
				[
					4,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[
					"cdkConnectedOverlay",
					"",
					"cdkConnectedOverlayLockPosition",
					"",
					3,
					"cdkConnectedOverlayOrigin",
					"cdkConnectedOverlayPositions",
					"cdkConnectedOverlayOpen"
				],
				[
					"cdkTrapFocus",
					"",
					"id",
					"account-switcher",
					1,
					"ms-account-switcher-panel",
					3,
					"keydown.escape"
				],
				[
					"id",
					"account-switcher-first-focus",
					"tabindex",
					"-1"
				],
				[1, "header"],
				[
					"src",
					"https://www.gstatic.com/images/branding/googlelogo/svg/googlelogo_clr_74x24px.svg",
					"alt",
					"Google",
					"width",
					"74",
					"height",
					"24",
					1,
					"google-logo"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Close",
					3,
					"click",
					"ve",
					"veClick"
				],
				[
					"aria-hidden",
					"true",
					3,
					"iconName"
				],
				[1, "profile"],
				[1, "profile-card"],
				[1, "profile-row"],
				[1, "profile-info"],
				[1, "name"],
				[1, "email"],
				[1, "badge-container"],
				[1, "actions-section"],
				[
					"ms-button",
					"",
					"variant",
					"as-primary",
					"size",
					"large",
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"manage-membership-button",
					3,
					"href",
					"ve",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-primary",
					"size",
					"large",
					1,
					"switch-account-button",
					3,
					"click",
					"ve",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-primary",
					"size",
					"large",
					1,
					"signout-button",
					3,
					"click",
					"ve",
					"veClick"
				],
				[1, "policy"],
				[1, "policy-label"],
				[
					"id",
					"privacyPolicyLink",
					3,
					"href"
				],
				["aria-hidden", "true"],
				[
					"id",
					"tosLink",
					3,
					"href"
				],
				[
					1,
					"using-key-badge",
					"hide-circle"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-secondary",
					"size",
					"large",
					1,
					"switch-subscription-button",
					3,
					"ve",
					"veClick",
					"disabled",
					"matTooltip"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-primary",
					"size",
					"large",
					1,
					"change-key-button",
					3,
					"click",
					"ve",
					"veClick",
					"disabled",
					"matTooltip"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-secondary",
					"size",
					"large",
					1,
					"switch-subscription-button",
					3,
					"click",
					"ve",
					"veClick",
					"disabled",
					"matTooltip"
				],
				[
					"ms-button",
					"",
					"variant",
					"as-primary",
					"size",
					"large",
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"manage-membership-button",
					3,
					"click",
					"href",
					"ve",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					1,
					"signin-button",
					3,
					"click",
					"ve",
					"veClick"
				],
				[
					3,
					"size",
					"src",
					"alt"
				],
				[
					"aria-label",
					"Profile photo",
					"role",
					"img",
					3,
					"initials",
					"size"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, P0b, 7, 14, "div", 6)(1, Q0b, 2, 3, "button", 7), _.z(2, T0b, 2, 1, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.NIb() ? 0 : 1);
				}
			},
			dependencies: [
				_.QK,
				_.Yy,
				_.GB,
				_.FB,
				_.JA,
				_.dz,
				_.HC,
				_.RK,
				_.nz,
				_.Bz,
				_.aF
			],
			styles: [".ms-account-switcher-panel[_ngcontent-%COMP%]{backdrop-filter:blur(80px);-webkit-backdrop-filter:blur(80px);background:color-mix(in srgb,var(--color-v3-surface-container-high) 50%,transparent);border:none;border-radius:20px;box-shadow:0 16px 32px -8px rgba(0,0,0,.4);-moz-box-sizing:border-box;box-sizing:border-box;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;padding:8px;position:absolute;right:20px;width:295px;-webkit-animation:_ngcontent-%COMP%_account-switcher-open .2s cubic-bezier(.2,0,0,1) forwards;animation:_ngcontent-%COMP%_account-switcher-open .2s cubic-bezier(.2,0,0,1) forwards;-webkit-transform-origin:top right;transform-origin:top right}@-webkit-keyframes _ngcontent-%COMP%_account-switcher-open{0%{opacity:0;-webkit-transform:scale(.95) translateY(-4px);transform:scale(.95) translateY(-4px)}to{opacity:1;-webkit-transform:scale(1) translateY(0);transform:scale(1) translateY(0)}}@keyframes _ngcontent-%COMP%_account-switcher-open{0%{opacity:0;-webkit-transform:scale(.95) translateY(-4px);transform:scale(.95) translateY(-4px)}to{opacity:1;-webkit-transform:scale(1) translateY(0);transform:scale(1) translateY(0)}}.avatar-ring-wrapper[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;border-radius:50%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:40px}.button-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;height:42px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;position:relative;width:42px}.button-container[_ngcontent-%COMP%]   connect-avatar[_ngcontent-%COMP%]{height:36px;width:36px}.button-container[_ngcontent-%COMP%]   connect-monogram[_ngcontent-%COMP%]{height:36px;width:36px}.button-container[_ngcontent-%COMP%]:focus, .button-container[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-hover);border-radius:50%;outline:none}.button-container[_ngcontent-%COMP%] > .g1-ring[_ngcontent-%COMP%]{width:24px;height:24px}.container[_ngcontent-%COMP%]{position:relative}.divider[_ngcontent-%COMP%]{background:var(--color-v3-outline);width:100%}.email[_ngcontent-%COMP%]{color:var(--color-v3-text-var);font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;letter-spacing:.2px;overflow:hidden;text-overflow:ellipsis;text-transform:none;white-space:nowrap}.g1-ring[_ngcontent-%COMP%]{aspect-ratio:1;background-clip:padding-box,border-box;border-radius:50%;background-image:-webkit-gradient(linear,left top,left bottom,from(var(--color-v3-surface)),to(var(--color-v3-surface))),conic-gradient(from 90deg,#3174f1 0deg,#3174f1 44.21deg,#249a41 44.63deg,#249a41 153.57deg,#f6ad01 153.6deg,#f6ad01 208.16deg,#e92d18 208.19deg,#e92d18 312.86deg,#3174f1 313.17deg,#3174f1 1turn);background-image:-webkit-linear-gradient(var(--color-v3-surface),var(--color-v3-surface)),conic-gradient(from 90deg,#3174f1 0deg,#3174f1 44.21deg,#249a41 44.63deg,#249a41 153.57deg,#f6ad01 153.6deg,#f6ad01 208.16deg,#e92d18 208.19deg,#e92d18 312.86deg,#3174f1 313.17deg,#3174f1 1turn);background-image:linear-gradient(var(--color-v3-surface),var(--color-v3-surface)),conic-gradient(from 90deg,#3174f1 0deg,#3174f1 44.21deg,#249a41 44.63deg,#249a41 153.57deg,#f6ad01 153.6deg,#f6ad01 208.16deg,#e92d18 208.19deg,#e92d18 312.86deg,#3174f1 313.17deg,#3174f1 1turn);background-origin:border-box;border:2px solid transparent;-moz-box-sizing:border-box;box-sizing:border-box}.header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-moz-box-sizing:border-box;box-sizing:border-box;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:12px 12px 0;position:relative;width:100%}.header[_ngcontent-%COMP%]   button[_ngcontent-%COMP%]{position:absolute;right:4px;top:4px}.google-logo[_ngcontent-%COMP%]{height:24px;width:auto}.name[_ngcontent-%COMP%]{color:var(--color-v3-text);font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;letter-spacing:.1px;overflow:hidden;text-overflow:ellipsis;text-transform:none;white-space:nowrap}.policy[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px;padding-bottom:4px;width:100%}.policy[_ngcontent-%COMP%]   span[_ngcontent-%COMP%]{color:var(--color-v3-outline);font-weight:700}.policy-label[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;text-align:center}.policy-label[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{border-radius:9999px;color:var(--color-v3-text-var);font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;padding:4px 8px;text-decoration:none;-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.policy-label[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]:focus, .policy-label[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-outline-var);outline:none}.profile[_ngcontent-%COMP%]{-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px;padding:12px;width:100%}.profile-card[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;width:100%}.profile-info[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;min-width:0}.profile-row[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;width:100%}.profile-row[_ngcontent-%COMP%]   connect-avatar[_ngcontent-%COMP%], .profile-row[_ngcontent-%COMP%]   connect-monogram[_ngcontent-%COMP%]{height:32px;width:32px}.badge-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:start;-webkit-justify-content:flex-start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:flex-start}.using-key-badge[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;background-color:var(--color-v3-button-container-accent);color:var(--color-v3-text-on-button);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;white-space:nowrap;text-transform:uppercase}.using-key-badge[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.using-key-badge.enabled[_ngcontent-%COMP%]:before, .using-key-badge.green[_ngcontent-%COMP%]:before, .using-key-badge.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.using-key-badge.gray[_ngcontent-%COMP%]:before, .using-key-badge.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.using-key-badge.confidential[_ngcontent-%COMP%]:before, .using-key-badge.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.using-key-badge.blue[_ngcontent-%COMP%]:before, .using-key-badge.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.using-key-badge.alert[_ngcontent-%COMP%]:before, .using-key-badge.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.using-key-badge.hide-circle[_ngcontent-%COMP%]:before{display:none}.actions-section[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;width:100%}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

