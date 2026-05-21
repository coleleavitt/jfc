"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		_.k1 = class {
			constructor() {
				this.YRb = "https://www.gstatic.com/aistudio/icons/spark_icon_traces.svg";
			}
		};
		_.k1.J = function(a) {
			return new (a || _.k1)();
		};
		_.k1.ka = _.u({
			type: _.k1,
			da: [["ms-sparkle-icon"]],
			ha: 1,
			ia: 1,
			la: [[
				"alt",
				"",
				3,
				"src"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.Fh(0, "img", 0);
				}
				if (a & 2) {
					_.Ch("src", b.YRb, _.rg);
				}
			},
			styles: ["img[_ngcontent-%COMP%]{height:100%;object-fit:contain}"]
		});
		var vjd;
		tjd = function(a, b) {
			var c = new Set(a.Ap());
			return b.filter((d) => !c.has(d));
		};
		_.wjd = function(a, b) {
			a = tjd(a, b).map((c) => ujd.get(c)).filter((c) => c !== undefined);
			if (a.length === 0) return null;
			a = a.join(", ");
			return {
				errorMessage: new _.xd("Missing permissions: {MISSING_PERMISSIONS}. Please contact your project administrator for assistance.").format({ MISSING_PERMISSIONS: a }),
				actions: [{
					link: vjd,
					text: "Learn more",
					Sq: true
				}]
			};
		};
		_.Ajd = function(a) {
			return _.mj(a, _.xjd, 1, _.oj()).reduce((b, c) => {
				c = _.Z(c, _.yjd, 2);
				var d;
				c = (d = (c == null ? undefined : _.Ys(c, _.Ls(c, _.zjd, 1))) || (c == null ? undefined : _.zo(c, 2, _.zjd))) != null ? d : 0;
				return b + Number(c);
			}, 0);
		};
		_.Bjd = function(a, b) {
			return _.Uc(a, 8, b);
		};
		_.Cjd = function(a, b) {
			return _.an(a, 10, b);
		};
		_.Djd = function(a, b) {
			return _.an(a, 12, b);
		};
		_.yjd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.zjd = [1, 2];
		_.xjd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		Ejd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getMethod() {
				return _.l(this, 4);
			}
		};
		_.j3 = function(a) {
			return _.mj(a, Ejd, 1, _.oj());
		};
		_.Fjd = function(a, b) {
			return _.$q(a.A, a.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/FetchMetricTimeSeries", b, {}, _.U4a);
		};
		vjd = (0, _.Kj)`https://docs.cloud.google.com/iam/docs/roles-permissions`;
		ujd = new Map([[14, "cloudquotas.quotas.get"], [13, "monitoring.timeSeries.list"]]);
		Gjd = [13];
		_.Hjd = function(a, b, c, d) {
			var e = "";
			var f = [];
			var g = undefined;
			if (b = _.wjd(b, Gjd)) {
				e = b.errorMessage, f = b.actions, g = 299870;
			} else {
				c.includes(0) ? (e = "Project quota tier unavailable. Please contact your project administrator for assistance.", g = 299909) : c.includes(1) && (e = "Project quota tier unknown. Please check back later.", g = 313565);
			}
			if (e) throw d && _.om(a.F, {
				content: e,
				Ne: "error",
				actions: f,
				S1: g
			}), Error(e);
		};
		_.Ijd = function(a, b, c, d) {
			if (_.pn(b) && d) throw _.om(a.F, {
				content: "Permission denied. Please contact your project administrator for assistance.",
				Ne: "error",
				S1: 299870
			}), Error("Permission denied. Please contact your project administrator for assistance.");
			if (b instanceof _.nn && b.code === 13) {
				_.Mw(a.reportError, Error("mi`" + c.getDisplayName(), { cause: b }));
			}
		};
		_.Jjd = function(a, b, c, d, e, f, g, k = []) {
			return _.x(function* () {
				if (b.length === 0) throw Error("ki");
				if (f == null || !f.length) throw Error("li");
				_.Hjd(a, d, f, true);
				try {
					var p = new _.Tx();
					var r = _.an(p, 11, b).Soa(c);
					var v = _.ot(r, 7, e);
					var w = _.Bjd(v, d.getName());
					var D = _.ot(w, 9, g);
					let G = _.Djd(_.Cjd(D, f), k);
					return yield _.Fjd(a.A, G);
				} catch (G) {
					throw _.Ijd(a, G, d, true), G;
				}
			});
		};
		_.k3 = class {
			constructor() {
				this.A = _.m(_.Zq);
				this.reportError = _.m(_.Nw);
				this.F = _.m(_.gC);
			}
		};
		_.k3.J = function(a) {
			return new (a || _.k3)();
		};
		_.k3.sa = _.Cd({
			token: _.k3,
			factory: _.k3.J,
			wa: "root"
		});
		_.Kjd = function() {
			return _.Jn().location.hostname.includes("aistudio-dev.corp.google.com");
		};
		var Mjd;
		Ljd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 14);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.rz());
				});
				_.Mh(1, 1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null)("ve", a.uMa().Y9a)("veClick", true)("veImpression", true);
			}
		};
		Njd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 15);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					_.Rn(c.dPa, "API", "Clicked Activate billing button on a Project");
					Mjd(c, "61cf5150-677c-4f21-9d62-6edc44b0a46b");
					return _.t();
				});
				_.Mh(1, 2);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null)("ve", a.uMa().uTa)("veClick", true)("veImpression", true);
			}
		};
		Ojd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 16);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					if (c.YEb()) {
						Mjd(c);
					} else {
						c.jna();
					}
					return _.t();
				});
				_.Mh(1, 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.P("disabled", a.Hb), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null);
			}
		};
		Pjd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 17);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.jna());
				});
				_.Mh(1, 4);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("title", a.XUa()), _.y(), _.Qh(a.XUa()), _.Rh(1);
			}
		};
		Qjd = function(a) {
			if (a & 1) {
				_.F(0, "span", 12), _.Gh(1), _.Mh(2, 5), _.Hh(), _.H();
			}
			if (a & 2) {
				a = _.K();
				let b = _.O(9);
				_.E("xapInlineDialog", b);
				_.y(2);
				_.Qh(a.fUb());
				_.Rh(2);
			}
		};
		Rjd = function(a) {
			if (a & 1) {
				_.R(0);
			}
			if (a & 2) {
				a = _.K(), _.S(" · ", a.YUa(), " ");
			}
		};
		Sjd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 18)(1, "div");
				_.Kh(2, 6);
				_.F(3, "a", 19);
				_.I(4, "span", 20);
				_.H();
				_.Lh();
				_.H();
				_.F(5, "div", 21);
				_.Gh(6, 21);
				_.Mh(7, 7);
				_.Hh();
				_.F(8, "button", 22);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Dxa());
				});
				_.H()()();
			}
			if (a & 2) {
				a = _.K(), _.y(4), _.E("iconName", a.S.Dk), _.y(3), _.Qh(a.billingAccountId()), _.Rh(7), _.y(), _.E("iconName", a.S.HOa);
			}
		};
		Tjd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.R(1, ","), _.H();
			}
		};
		Vjd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 3);
				_.J("click", function() {
					var d = _.q(c).V;
					var e = _.K(2);
					return _.t(e.jna(d));
				});
				_.R(1);
				_.H();
				_.B(2, Tjd, 2, 0, "span");
			}
			if (a & 2) {
				a = b.V;
				let c = b.jb;
				b = b.lj;
				let d = _.K(2);
				_.E("ve", d.ve.Mob)("veClick", true)("veImpression", true);
				_.y();
				_.S(" ", Ujd(d, a.getDisplayName()), " ");
				_.y();
				_.C(c !== b - 1 ? 2 : -1);
			}
		};
		Xjd = function(a) {
			if (a & 1) {
				_.F(0, "ms-callout", 0)(1, "div", 1)(2, "span"), _.R(3), _.H(), _.F(4, "div", 2), _.Ah(5, Vjd, 3, 5, null, null, Wjd), _.H()()();
			}
			if (a & 2) {
				a = _.K(), _.y(3), _.U(a.calloutMessage()), _.y(2), _.Bh(a.W_());
			}
		};
		Mjd = function(a, b) {
			a.dialog.open(_.MG, {
				id: "oaas-dialog",
				data: {
					experienceId: b,
					st: a.project()
				}
			});
		};
		_.l3 = class {
			constructor() {
				this.project = _.Li.required();
				this.uMa = _.Li.required();
				this.dPa = _.m(_.Ou);
				this.R = _.m(_.GG);
				this.dialog = _.m(_.rC);
				this.U = _.m(_.SC);
				this.I = _.m(_.iC);
				this.aa = _.m(_.Cl);
				this.X = _.m(_.Qu);
				this.S = _.Dk;
				this.Ul = _.OG;
				this.Hb = _.Nn(this.X.Oe);
				_.W(() => "You do not have permission to view this account. Contact your billing admin to proceed.");
				this.H = _.W(() => _.au(this.project()));
				this.F = _.W(() => {
					var a;
					return (a = this.H()) == null ? undefined : a.gk();
				});
				this.A = _.W(() => {
					var a;
					return this.R.U().get((a = this.F()) != null ? a : "");
				});
				this.bBa = _.W(() => !!this.A());
				this.billingAccountId = _.W(() => {
					var a = this.F();
					return a ? _.Pn(a) : "";
				});
				this.XUa = _.W(() => this.A() ? this.A().getDisplayName() : "Billing account");
				this.fUb = _.W(() => {
					var a = this.billingAccountId();
					return `...${a.substring(a.length - 4)}`;
				});
				this.LH = _.W(() => !!this.F());
				this.YGb = _.W(() => {
					var a;
					if (a = !!this.A() && this.A().RY() !== 1) {
						a = this.A();
						a = _.Lm(a, 12) === 2;
					}
					return a;
				});
				this.YEb = _.W(() => {
					var a;
					var b;
					var c;
					return (c = (a = this.A()) == null ? undefined : (b = a.Ap()) == null ? undefined : b.includes(2)) != null ? c : false;
				});
				this.UAa = _.W(() => {
					var a;
					return ((a = this.H()) == null ? undefined : _.Lm(a, 3)) === 1;
				});
				this.oNb = _.W(() => {
					if (this.UAa()) return "Free trial";
					var a;
					return _.iya((a = this.project().ah()) != null ? a : 0);
				});
				this.YUa = _.W(() => {
					if (!this.A()) return "";
					var a;
					if (this.UAa() || ((a = this.H()) == null ? undefined : _.Lm(a, 3)) === 2) return "";
					a = this.A().RY();
					switch (a) {
						case 1: return "Prepay";
						case 2: return "Postpay";
						case 0: return "";
						default: _.sb(a, undefined);
					}
				});
			}
			rz() {
				_.Rn(this.dPa, "API", "Clicked Set up billing button on a Project");
				Mjd(this);
			}
			jna() {
				this.aa.navigate(["billing"], { queryParams: { billing: this.billingAccountId() } });
			}
			Dxa() {
				var a = this.U.copy(this.billingAccountId());
				var b = a ? "Copied to clipboard" : "Error copying to clipboard";
				if (a) {
					this.I.success(b);
				} else {
					this.I.error(b);
				}
			}
		};
		_.l3.J = function(a) {
			return new (a || _.l3)();
		};
		_.l3.ka = _.u({
			type: _.l3,
			da: [["ms-quota-tier-cell"]],
			inputs: {
				project: [1, "project"],
				uMa: [1, "veMap"]
			},
			ha: 10,
			ia: 3,
			la: () => [
				["permissionDeniedDialogTemplate", ""],
				" Set up billing ",
				" Activate billing ",
				" Set up prepay ",
				" �0� ",
				" Account (�0�) ",
				" You don't have sufficient read permissions for this billing account. Learn more about �#3� billing permissions �#4��/#4��/#3�",
				" Billing account ID: �0� ",
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-billing-link",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-activate-billing-button",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-prepay-button",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View billing",
					"data-test-billing-account-name-button",
					"",
					1,
					"link",
					"billing-account-name-link",
					3,
					"title"
				],
				[
					"dialogLabel",
					"Billing permission error dialog",
					"data-test-billing-account-name-button-no-permission",
					"",
					1,
					"disabled",
					3,
					"xapInlineDialog"
				],
				[
					"data-test-quota-tier-text",
					"",
					1,
					"sub-text"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-billing-link",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-activate-billing-button",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-prepay-button",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View billing",
					"data-test-billing-account-name-button",
					"",
					1,
					"link",
					"billing-account-name-link",
					3,
					"click",
					"title"
				],
				[1, "permission-denied-dialog-content"],
				[
					"href",
					"https://docs.cloud.google.com/billing/docs/how-to/grant-access-to-billing",
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"permission-denied-dialog-link"
				],
				[3, "iconName"],
				[1, "billing-account-id"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy billing account ID",
					"data-test-copy-billing-account-id",
					"",
					3,
					"click",
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Ljd, 2, 5, "button", 8)(1, Njd, 2, 5, "button", 9)(2, Ojd, 2, 4, "button", 10)(3, Pjd, 2, 2, "button", 11)(4, Qjd, 3, 2, "span", 12), _.F(5, "div", 13), _.R(6), _.B(7, Rjd, 1, 1), _.H(), _.z(8, Sjd, 9, 3, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.LH() ? b.UAa() ? 1 : b.YGb() ? 2 : b.bBa() ? 3 : 4 : 0), _.y(6), _.S(" ", b.oNb(), " "), _.y(), _.C(b.YUa() ? 7 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.IC,
				_.Bz,
				_.EC
			],
			styles: [".link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;cursor:pointer;padding:0;margin:0;border:none;height:24px}.sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.billing-account-name-link[_ngcontent-%COMP%]{display:inline-block;max-width:180px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;text-align:left}.disabled[_ngcontent-%COMP%]{color:var(--color-v3-text-disable)}.permission-denied-dialog-content[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;padding:16px}.permission-denied-dialog-content[_ngcontent-%COMP%]   .billing-account-id[_ngcontent-%COMP%], .permission-denied-dialog-content[_ngcontent-%COMP%]   .permission-denied-dialog-link[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.permission-denied-dialog-content[_ngcontent-%COMP%]   .billing-account-id[_ngcontent-%COMP%]{margin-top:8px}"]
		});
		var Ujd;
		Wjd = (a, b) => b.Ya();
		Ujd = function(a, b) {
			return b.length <= 10 || a.W_().length === 1 || a.W_().length < 3 && !a.I.A.small() ? b : b.substring(0, 10) + "...";
		};
		_.m3 = class {
			constructor() {
				this.ve = { Mob: 311861 };
				this.F = _.m(_.GG);
				this.Za = _.m(_.Iy);
				this.H = _.m(_.Cl);
				this.I = _.m(_.ZC);
				this.r$a = _.M(false);
				this.Sd = this.Za.Sd;
				this.A = this.F.A;
				this.W_ = _.W(() => this.Sd().filter((a) => {
					var b;
					a = (b = _.au(a)) == null ? undefined : b.gk();
					if (!a) return false;
					var c;
					var d;
					return (d = (c = this.A().get(a)) == null ? undefined : c.has("PREPAY_PAYMENT_ACTION_NEEDED")) != null ? d : false;
				}));
				this.calloutMessage = _.W(() => new _.xd("{numProjects, plural, =1 {Your project has issues needing your attention.}other {Your projects have issues needing your attention.}}").format({ numProjects: this.W_().length }));
				_.Fk([this.W_], () => {
					this.r$a.set(this.W_().length > 0);
				});
			}
			jna(a) {
				var b = { queryParams: {} };
				var c;
				if (a = (c = _.au(a)) == null ? undefined : c.gk()) {
					b.queryParams.billing = _.Pn(a);
				}
				this.H.navigate(["billing"], b);
			}
		};
		_.m3.J = function(a) {
			return new (a || _.m3)();
		};
		_.m3.ka = _.u({
			type: _.m3,
			da: [["ms-payment-alert-callout"]],
			ha: 1,
			ia: 1,
			la: [
				[
					"calloutType",
					"error",
					1,
					"callout"
				],
				[
					"callout-content",
					"",
					1,
					"callout-content"
				],
				[1, "project-links"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"project-link",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Xjd, 7, 1, "ms-callout", 0);
				}
				if (a & 2) {
					_.C(b.r$a() ? 0 : -1);
				}
			},
			dependencies: [
				_.zA,
				_.Yy,
				_.Cz,
				_.Bz
			],
			styles: [".callout[_ngcontent-%COMP%]{margin-bottom:20px}.project-links[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px;-webkit-box-align:end;-webkit-align-items:flex-end;-moz-box-align:end;-ms-flex-align:end;align-items:flex-end;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;margin-top:6px}.project-link[_ngcontent-%COMP%]{height:20px;padding:0}"]
		});
		_.Yjd = function(a) {
			var b = a.replaceAll("-", " ").replace(/\b\w/g, (c) => c.toUpperCase()).replace(/\b(tts|er)\b/gi, (c) => c.toUpperCase()).replace(/(\d+)\.0/g, "$1").replace(/\b(\d\w*[bm])\b/gi, (c) => c.toUpperCase());
			return a === "gemini-3.1-flash-image" ? `Nano Banana 2 (${b})` : a === "gemini-3-pro-image" ? `Nano Banana Pro (${b})` : a === "gemini-2.5-flash-preview-image" ? `Nano Banana (${b})` : b;
		};
		Zjd = function(a) {
			if (a & 1) {
				_.I(0, "ms-sparkle-icon");
			}
		};
		$jd = function(a) {
			if (a & 1) {
				_.F(0, "a", 10), _.Mh(1, 0), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("ve", a.ve.xta)("veClick", true)("veImpression", true);
			}
		};
		akd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 12);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.ZE());
				});
				_.Mh(1, 1);
				_.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("ve", a.ve.wta)("veClick", true)("veImpression", true);
			}
		};
		bkd = function(a) {
			if (a & 1) {
				_.B(0, $jd, 2, 3, "a", 10)(1, akd, 2, 3, "button", 11);
			}
			if (a & 2) {
				a = _.K(), _.C(a.Hb ? 0 : 1);
			}
		};
		ckd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 13);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.vr(true));
				});
				_.Mh(1, 2);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("ve", a.ve.yta)("veClick", true)("veImpression", true);
			}
		};
		dkd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "ms-import-projects-panel", 14);
				_.J("onClose", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.vr(false));
				});
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("isRightPanelOpen", a.Jq());
			}
		};
		ekd = [[[
			"",
			"empty-state-buttons",
			""
		]]];
		_.n3 = class {
			constructor() {
				this.ve = {
					wta: 278932,
					xta: 278935,
					yta: 278936
				};
				this.H7a = _.Ki();
				this.F = _.m(_.Qu);
				this.dialog = _.m(_.rC);
				this.A = _.m(_.Ou);
				this.headline = _.Li.required();
				this.message = _.Li.required();
				this.SB = _.Li.required();
				this.V$a = _.Li.required();
				this.WJa = _.Li.required();
				this.v$a = _.Li.required();
				this.Hb = _.Nn(this.F.Oe);
				this.Jq = _.M(false);
				this.Ymb = "Learn more";
			}
			vr(a) {
				if (a) {
					_.Rn(this.A, "API", "Clicked Import Projects Button in project zero state");
				}
				this.Jq.set(a);
			}
			ZE() {
				var a = this;
				return _.x(function* () {
					_.Rn(a.A, "API", "Clicked Create Project Button in project zero state");
					if (yield _.pf(_.jC(a.dialog.open(_.tE)))) {
						a.H7a.emit();
					}
				});
			}
		};
		_.n3.J = function(a) {
			return new (a || _.n3)();
		};
		_.n3.ka = _.u({
			type: _.n3,
			da: [["ms-project-zero-state"]],
			inputs: {
				headline: [1, "headline"],
				message: [1, "message"],
				SB: [1, "learnMoreUrl"],
				V$a: [1, "showSparkle"],
				WJa: [1, "showImportProjectsButton"],
				v$a: [1, "showCreateProjectButton"]
			},
			outputs: { H7a: "projectCreated" },
			fc: ["[empty-state-buttons]"],
			ha: 13,
			ia: 8,
			la: () => [
				" Create a new project in GCP ",
				" Create project ",
				" Import projects ",
				[1, "empty-state-container"],
				[1, "empty-state-headline"],
				[1, "empty-state-text"],
				[
					"target",
					"_blank",
					3,
					"href"
				],
				[1, "empty-state-buttons"],
				[
					"ms-button",
					"",
					"size",
					"large",
					"data-test-id",
					"import-projects-button",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[3, "isRightPanelOpen"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"href",
					"http://go/aistudio-apikey#create-gcp-project",
					"target",
					"_blank",
					"matTooltip",
					"Google Internal Only",
					"matTooltipPosition",
					"above",
					"aria-label",
					"Create a new project in GCP",
					1,
					"custom-link",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"data-test-id",
					"import-projects-button",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					3,
					"onClose",
					"isRightPanelOpen"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(ekd), _.F(0, "div", 3), _.B(1, Zjd, 1, 0, "ms-sparkle-icon"), _.F(2, "p", 4), _.R(3), _.H(), _.F(4, "p", 5), _.R(5), _.F(6, "a", 6), _.R(7), _.H()(), _.F(8, "div", 7), _.B(9, bkd, 2, 1), _.B(10, ckd, 2, 3, "button", 8), _.Yh(11), _.H()(), _.B(12, dkd, 1, 1, "ms-import-projects-panel", 9);
				}
				if (a & 2) {
					_.y(), _.C(b.V$a() ? 1 : -1), _.y(2), _.U(b.headline()), _.y(2), _.S(" ", b.message(), " "), _.y(), _.E("href", b.SB(), _.rg), _.y(), _.U(b.Ymb), _.y(2), _.C(b.v$a() ? 9 : -1), _.y(), _.C(b.WJa() ? 10 : -1), _.y(2), _.C(b.WJa() ? 12 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.rE,
				_.IC,
				_.HC,
				_.k1,
				_.Bz
			],
			styles: [".empty-state-container[_ngcontent-%COMP%]{max-width:400px;margin:0 auto;padding:32px 0;text-align:center}.empty-state-headline[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text);margin-top:36px;margin-bottom:8px}ms-sparkle-icon[_ngcontent-%COMP%] + .empty-state-headline[_ngcontent-%COMP%]{margin-top:8px}.empty-state-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var);margin-bottom:8px}.empty-state-buttons[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;gap:8px;margin-top:36px}.custom-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}"]
		});
		_.hr("N168Pd");
		var hkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 31), _.I(1, "mat-spinner", 32), _.H();
			}
		};
		var jkd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 47);
				_.J("click", function() {
					_.q(b);
					var c = _.K(3);
					return _.t(ikd(c));
				});
				_.Mh(1, 8);
				_.H();
			}
			if (a & 2) {
				a = _.K(3), _.E("disabled", a.jVa())("matTooltip", a.jVa() ? a.Smb : "")("ve", a.ve.Rfb)("veClick", true)("veImpression", true);
			}
		};
		var kkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 43), _.I(1, "mat-spinner", 48), _.H();
			}
		};
		var lkd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.Mh(1, 9), _.H();
			}
		};
		var mkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 44), _.I(1, "span", 49), _.F(2, "span"), _.Mh(3, 10), _.H()();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.S.WARNING);
			}
		};
		var nkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 45), _.I(1, "span", 49), _.F(2, "span"), _.Mh(3, 11), _.H()();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.S.ERROR);
			}
		};
		var okd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var qkd = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 46)(1, "div", 50), _.I(2, "ms-api-key-key-string", 51), _.F(3, "div", 52), _.R(4), _.H()(), _.F(5, "div", 53), _.Mh(6, 12), _.H(), _.F(7, "div", 54), _.z(8, okd, 1, 0, "ng-container", 55), _.H()();
			}
			if (a & 2) {
				a = b.V, _.K(3), b = _.O(8), _.y(2), _.E("apiKey", a.key), _.y(2), _.U(a.key.getDisplayName()), _.y(4), _.E("ngTemplateOutlet", b)("ngTemplateOutletContext", _.Ai(4, pkd, a));
			}
		};
		var skd = function(a) {
			if (a & 1) {
				_.F(0, "div", 36)(1, "div", 37), _.I(2, "span", 38), _.F(3, "span"), _.Mh(4, 6), _.H(), _.I(5, "span", 39), _.F(6, "span", 40), _.Mh(7, 7), _.H()(), _.F(8, "div", 41), _.B(9, jkd, 2, 5, "button", 42)(10, kkd, 2, 0, "div", 43)(11, lkd, 2, 0, "span")(12, mkd, 4, 1, "div", 44)(13, nkd, 4, 1, "div", 45), _.H()(), _.Ah(14, qkd, 9, 6, "div", 46, rkd);
			}
			if (a & 2) {
				let b;
				a = _.K(2);
				_.y(2);
				_.E("iconName", a.S.Fk);
				_.y(3);
				_.E("iconName", a.S.Bf);
				_.y(2);
				_.Qh(a.DY().length)(a.DY().length);
				_.Rh(7);
				_.y(2);
				_.C((b = a.XW()) === a.t2.M2 ? 9 : b === a.t2.Eta ? 10 : b === a.t2.W2 ? 11 : b === a.t2.iob ? 12 : b === a.t2.fea ? 13 : -1);
				_.y(5);
				_.Bh(a.DY());
			}
		};
		var tkd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var ukd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var wkd = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 46)(1, "div", 50), _.I(2, "ms-api-key-key-string", 51), _.F(3, "div", 58), _.R(4), _.H()(), _.F(5, "div", 53)(6, "div", 59), _.Mh(7, 15), _.H(), _.z(8, tkd, 1, 0, "ng-container", 55), _.H(), _.F(9, "div", 60)(10, "a", 61), _.Mh(11, 16), _.H(), _.z(12, ukd, 1, 0, "ng-container", 55), _.H()();
			}
			if (a & 2) {
				a = b.V;
				b = _.K(3);
				let c = _.O(8);
				let d = _.O(10);
				let e = _.O(12);
				_.y(2);
				_.E("apiKey", a.key);
				_.y();
				_.E("matTooltip", a.key.getDisplayName());
				_.y();
				_.U(a.key.getDisplayName());
				_.y(4);
				_.E("ngTemplateOutlet", d)("ngTemplateOutletContext", _.Bi(12, vkd, a, e));
				_.y(2);
				_.E("iconName", b.S.Ps)("isIconPositionEnd", true)("ve", b.ve.Tfb)("veClick", true)("veImpression", true);
				_.y(2);
				_.E("ngTemplateOutlet", c)("ngTemplateOutletContext", _.Ai(15, pkd, a));
			}
		};
		var xkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 56)(1, "div", 37), _.I(2, "span", 38), _.F(3, "span"), _.Mh(4, 13), _.H(), _.I(5, "span", 57), _.F(6, "span", 40), _.Mh(7, 14), _.H()()(), _.Ah(8, wkd, 13, 17, "div", 46, rkd);
			}
			if (a & 2) {
				a = _.K(2), _.y(2), _.E("iconName", a.S.rqb), _.y(3), _.E("iconName", a.S.Bf), _.y(2), _.Qh(a.sma().length)(a.sma().length), _.Rh(7), _.y(), _.Bh(a.sma());
			}
		};
		var ykd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.Mh(1, 20), _.H();
			}
		};
		var zkd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Akd = function(a) {
			if (a & 1) {
				_.F(0, "div", 59), _.Mh(1, 21), _.H(), _.z(2, zkd, 1, 0, "ng-container", 55);
			}
			if (a & 2) {
				a = _.K().V;
				_.K(3);
				let b = _.O(10);
				let c = _.O(14);
				_.y(2);
				_.E("ngTemplateOutlet", b)("ngTemplateOutletContext", _.Bi(2, vkd, a, c));
			}
		};
		var Bkd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Ckd = function(a) {
			if (a & 1) {
				_.z(0, Bkd, 1, 0, "ng-container", 55);
			}
			if (a & 2) {
				a = _.K().V;
				_.K(3);
				let b = _.O(8);
				_.E("ngTemplateOutlet", b)("ngTemplateOutletContext", _.Ai(2, pkd, a));
			}
		};
		var Ekd = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 46)(1, "div", 50), _.I(2, "ms-api-key-key-string", 51), _.F(3, "div", 58), _.R(4), _.H()(), _.F(5, "div", 53), _.B(6, ykd, 2, 0, "span")(7, Akd, 3, 5), _.H(), _.F(8, "div", 60)(9, "a", 61), _.Mh(10, 19), _.H(), _.B(11, Ckd, 1, 4, "ng-container"), _.H()();
			}
			if (a & 2) {
				let c;
				a = b.V;
				b = _.K(3);
				_.y(2);
				_.E("apiKey", a.key);
				_.y();
				_.E("matTooltip", a.key.getDisplayName());
				_.y();
				_.U(a.key.getDisplayName());
				_.y(2);
				_.C(a.nFb ? 6 : 7);
				_.y(3);
				_.E("iconName", b.S.Ps)("isIconPositionEnd", true)("ve", b.ve.Ufb)("veClick", true)("veImpression", true);
				_.y(2);
				_.C(((c = _.Z(a.key, Dkd, 11)) == null ? 0 : _.Pm(c, 1)) ? -1 : 11);
			}
		};
		var Fkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 62)(1, "div", 37), _.I(2, "span", 38), _.F(3, "span"), _.Mh(4, 17), _.H(), _.I(5, "span", 63), _.F(6, "span", 40), _.Mh(7, 18), _.H()()(), _.Ah(8, Ekd, 12, 10, "div", 46, rkd);
			}
			if (a & 2) {
				a = _.K(2), _.y(2), _.E("iconName", a.S.sqb), _.y(3), _.E("iconName", a.S.Bf), _.y(2), _.Qh(a.Hma().length)(a.Hma().length), _.Rh(7), _.y(), _.Bh(a.Hma());
			}
		};
		var Gkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 33), _.Kh(1, 5), _.I(2, "a", 34), _.Lh(), _.H(), _.F(3, "div", 35), _.B(4, skd, 16, 5), _.B(5, xkd, 10, 4), _.B(6, Fkd, 10, 4), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(4), _.C(a.DY().length > 0 ? 4 : -1), _.y(), _.C(a.sma().length > 0 ? 5 : -1), _.y(), _.C(a.Hma().length > 0 ? 6 : -1);
			}
		};
		var Ikd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 66);
				_.J("click", function() {
					_.q(b);
					var c = _.K().V;
					var d = _.K();
					return _.t(Hkd(d, c));
				});
				_.Mh(1, 22);
				_.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("ve", a.ve.Sfb)("veClick", true)("veImpression", true);
			}
		};
		var Jkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 43), _.I(1, "mat-spinner", 48), _.H();
			}
		};
		var Kkd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.Mh(1, 23), _.H();
			}
		};
		var Lkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 65), _.I(1, "span", 49), _.F(2, "span"), _.Mh(3, 24), _.H()();
			}
			if (a & 2) {
				a = _.K(2), _.y(), _.E("iconName", a.S.ERROR);
			}
		};
		var Mkd = function(a, b) {
			if (a & 1) {
				_.B(0, Ikd, 2, 3, "button", 64)(1, Jkd, 2, 0, "div", 43)(2, Kkd, 2, 0, "span")(3, Lkd, 4, 1, "div", 65);
			}
			if (a & 2) {
				let c;
				a = b.V;
				b = _.K();
				_.C((c = a.dU()) === b.Iea.M2 ? 0 : c === b.Iea.Eta ? 1 : c === b.Iea.npb ? 2 : c === b.Iea.fkb ? 3 : -1);
			}
		};
		var Nkd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Okd = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 70), _.R(1), _.H();
			}
			if (a & 2) {
				a = b.V, _.y(), _.U(a);
			}
		};
		var Pkd = function(a) {
			if (a & 1) {
				_.F(0, "div", 69), _.Ah(1, Okd, 2, 1, "div", 70, _.zh), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.y(), _.Bh(a.services);
			}
		};
		var Qkd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "div", 67);
				_.J("click", function() {
					var d = _.q(c).V;
					_.K();
					d.vc.set(!d.vc());
					return _.t();
				})("keydown.enter", function() {
					var d = _.q(c).V;
					_.K();
					d.vc.set(!d.vc());
					return _.t();
				})("keydown.space", function(d) {
					var e = _.q(c).V;
					_.K();
					d.preventDefault();
					e.vc.set(!e.vc());
					return _.t();
				});
				_.z(1, Nkd, 1, 0, "ng-container", 55);
				_.I(2, "span", 68);
				_.H();
				_.B(3, Pkd, 3, 0, "div", 69);
			}
			if (a & 2) {
				a = b.V;
				b = b.kz;
				let c = _.K();
				_.wh("aria-expanded", a.vc());
				_.y();
				_.E("ngTemplateOutlet", b)("ngTemplateOutletContext", _.Ai(5, pkd, a));
				_.y();
				_.E("iconName", a.vc() ? c.S.iA : c.S.Ck);
				_.y();
				_.C(a.vc() ? 3 : -1);
			}
		};
		var Rkd = function(a, b) {
			if (a & 1) {
				_.Kh(0, 25), _.I(1, "span")(2, "span"), _.Lh();
			}
			if (a & 2) {
				a = b.V, _.y(2), _.Qh(a.z$)(a.z$), _.Rh(0);
			}
		};
		var Skd = function(a, b) {
			if (a & 1) {
				_.Kh(0, 26), _.I(1, "span"), _.Lh();
			}
			if (a & 2) {
				a = b.V, _.y(), _.Qh(a.z$)(a.z$), _.Rh(0);
			}
		};
		_.Uy.prototype.kda = _.ca(138, function(a, b = {}) {
			var c = this;
			return _.x(function* () {
				c.sO();
				try {
					let d = yield Yld(c, a, b);
					let e = c.F.get(_.$x(a));
					if (e) {
						c.F.set(_.$x(a), e.map((f) => f.getName() === d.getName() ? d : f));
					}
					if (!b.MRb) {
						c.H.reload();
					}
				} catch (d) {
					throw d instanceof Error && c.U.set(d), _.Rn(c.I, "API", "API Key Update Failed"), d;
				}
			});
		});
		var Dkd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var o3 = class {
			constructor() {
				this.dialog = _.m(_.rC);
				this.Klb = _.m(_.Ou);
				this.apiKey = _.Li.required();
			}
		};
		o3.J = function(a) {
			return new (a || o3)();
		};
		o3.ka = _.u({
			type: o3,
			da: [["ms-api-key-key-string"]],
			inputs: { apiKey: [1, "apiKey"] },
			ha: 3,
			ia: 3,
			la: [[
				"ms-button",
				"",
				"variant",
				"link",
				1,
				"key-string-link",
				3,
				"click"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "button", 0), _.J("click", function() {
						b.dialog.open(_.YC, { data: { key: b.apiKey() } });
						_.Rn(b.Klb, "API", "Open API Key Details Dialog");
					}), _.R(1), _.Ei(2, "maskApiKeyKeyString"), _.H();
				}
				if (a & 2) {
					_.y(), _.S(" ", _.Fi(2, 1, _.Io(b.apiKey())), "\n");
				}
			},
			dependencies: [_.Yy, _.aF],
			styles: [".key-string-link[_ngcontent-%COMP%]{all:unset;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-link);cursor:pointer}"]
		});
		var bmd = class {
			constructor() {
				this.Wa = _.m(_.kC);
				this.data = _.m(_.qC, { optional: true });
				this.ve = { Qfb: 325352 };
				var a;
				var b;
				this.C5 = _.M((b = (a = this.data) == null ? undefined : a.cla) != null ? b : false);
				this.S = _.Dk;
			}
			Ue() {
				this.Wa.close(false);
			}
			u_() {
				this.Wa.close(true);
			}
		};
		bmd.J = function(a) {
			return new (a || bmd)();
		};
		bmd.ka = _.u({
			type: bmd,
			da: [["ms-api-key-restriction-confirmation-dialog"]],
			ha: 17,
			ia: 8,
			la: () => [
				"Restrict to Gemini API only?",
				" Restricting to Gemini API only will �#9�immediately revoke�/#9� this key's access to any other Google Cloud services. Ensure no production apps rely on this key for other services. �#10�Learn more�/#10�",
				" Back ",
				" Restrict key ",
				"I understand this may break existing services using this key.",
				[1, "action-confirmation"],
				[
					"mat-dialog-title",
					"",
					1,
					"shared-dialog-header"
				],
				[1, "title"],
				[
					1,
					"warning-icon",
					3,
					"iconName"
				],
				["documentation-path", "/gemini-api/docs/api-key#secure-unrestricted-keys"],
				["align", "end"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-id",
					"cancel-button",
					3,
					"click"
				],
				[
					"ms-button",
					"",
					"data-test-id",
					"continue-button",
					3,
					"click",
					"disabled",
					"ve",
					"veClick",
					"veImpression"
				],
				[3, "change"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 5)(1, "div", 6)(2, "div", 7), _.B(3, fkd, 1, 1, "span", 8), _.F(4, "span"), _.Mh(5, 0), _.H()()(), _.F(6, "mat-dialog-content")(7, "p"), _.Kh(8, 1), _.I(9, "span")(10, "a", 9), _.Lh(), _.H(), _.B(11, gkd, 3, 0, "mat-checkbox"), _.H(), _.F(12, "mat-dialog-actions", 10)(13, "button", 11), _.J("click", function() {
						return b.Ue();
					}), _.Mh(14, 2), _.H(), _.F(15, "button", 12), _.J("click", function() {
						return b.u_();
					}), _.Mh(16, 3), _.H()()();
				}
				if (a & 2) {
					_.y(3), _.C((b.data == null ? 0 : b.data.cla) ? -1 : 3), _.y(6), _.P("warning-highlight", !(b.data == null ? 0 : b.data.cla)), _.y(2), _.C((b.data == null ? 0 : b.data.cla) ? -1 : 11), _.y(4), _.E("disabled", !b.C5())("ve", b.ve.Qfb)("veClick", true)("veImpression", true);
				}
			},
			dependencies: [
				_.Yy,
				_.LC,
				_.dz,
				_.qE,
				_.pE,
				_.xC,
				_.uC,
				_.wC,
				_.vC,
				_.Bz
			],
			styles: [".action-confirmation[_ngcontent-%COMP%]{max-width:500px}.shared-dialog-header[_ngcontent-%COMP%]   .title[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}mat-dialog-content[_ngcontent-%COMP%]   p[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;margin-bottom:8px}mat-dialog-actions[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end;gap:8px}.warning-highlight[_ngcontent-%COMP%], .warning-icon[_ngcontent-%COMP%]{color:var(--color-v3-accent-3)}\n/*# sourceMappingURL=api_key_restriction_confirmation_dialog.css.map */"]
		});
		var cmd = new Set([
			"generativelanguage.googleapis.com",
			"staging-generativelanguage.sandbox.googleapis.com",
			"test-generativelanguage.sandbox.googleapis.com"
		]);
		var p3 = class {
			constructor() {
				this.A = _.m(_.k3);
			}
		};
		p3.J = function(a) {
			return new (a || p3)();
		};
		p3.sa = _.Cd({
			token: p3,
			factory: p3.J,
			wa: "root"
		});
		var pkd = (a) => ({ V: a });
		var vkd = (a, b) => ({
			V: a,
			kz: b
		});
		var rkd = (a, b) => _.l(b.key, 10);
		var ikd = function(a) {
			return _.x(function* () {
				if (a.XW() === "not_started") {
					var b = a.DY().filter((c) => c.dU() === "not_started");
					if (b.length !== 0 && (yield hmd(a, true))) {
						a.XW.set("restricting"), b = yield Promise.all(b.map((c) => imd(a, c))), b.every((c) => c === true) ? (a.XW.set("success"), a.F.success("All API keys restricted successfully!")) : b.some((c) => c === true) ? (a.XW.set("partial_failure"), a.F.error("Some API keys failed to restrict.")) : (a.XW.set("failed"), a.F.error("Failed to restrict all API keys."));
					}
				}
			});
		};
		var Hkd = function(a, b) {
			return _.x(function* () {
				yield imd(a, b, {
					pr: true,
					nxb: true
				});
			});
		};
		var hmd = function(a, b) {
			return _.x(function* () {
				var c = a.dialog.open(bmd, { data: { cla: b } });
				return _.pf(_.jC(c));
			});
		};
		var imd = function(a, b, { pr: c = false, nxb: d = false } = {}) {
			return _.x(function* () {
				if (b.dU() !== "not_started" || d && !(yield hmd(a, b.EJ === "gemini_only"))) return false;
				b.dU.set("restricting");
				try {
					yield a.A.kda(b.key, {
						X8a: true,
						MRb: true
					});
					b.dU.set("restricted");
					if (c) {
						a.F.success("API key restricted successfully!");
					}
					return a.R = true;
				} catch (e) {
					b.dU.set("failed_to_restrict");
					if (c) {
						a.F.error("Failed to restrict API key.");
					}
					return false;
				}
			});
		};
		var q3 = class {
			constructor() {
				this.A = _.m(_.Uy);
				this.ve = {
					Rfb: 325331,
					Sfb: 325333,
					Tfb: 325332,
					Ufb: 325334
				};
				this.F = _.m(_.iC);
				this.U = _.m(p3);
				_.m(_.CG);
				this.dialog = _.m(_.rC);
				this.Wa = _.m(_.kC);
				this.data = _.m(_.qC);
				this.R = false;
				this.apiKey = this.data.apiKey;
				this.S = _.Dk;
				this.Iea = fmd;
				this.t2 = gmd;
				this.Smb = "All keys have been tried to be restricted.";
				this.I = _.Zi(Object.assign({}, {}, {
					params: () => ({
						keys: this.A.ea(),
						loading: this.A.ef()
					}),
					Xc: ({ params: a }) => {
						var b = this;
						return _.x(function* () {
							return a.loading || a.keys.length === 0 ? [] : emd(b.U, a.keys);
						});
					}
				}));
				this.H = _.W(() => {
					var a;
					return ((a = this.I.value()) != null ? a : []).map((b) => Object.assign({}, b, {
						nFb: b.EJ === "no_usage" || b.EJ === "unknown",
						dU: _.M("not_started"),
						vc: _.M(false)
					}));
				});
				this.DY = _.W(() => this.H().filter((a) => a.EJ === "gemini_only"));
				this.sma = _.W(() => this.H().filter((a) => a.EJ === "mixed_with_gemini"));
				this.Hma = _.W(() => this.H().filter((a) => a.EJ === "mixed_without_gemini" || a.EJ === "no_usage" || a.EJ === "unknown"));
				this.EMb = this.I.Sa;
				this.ef = this.A.ef;
				this.jVa = _.W(() => this.DY().every((a) => a.dU() !== "not_started"));
				this.XW = _.M("not_started");
				_.jC(this.Wa).subscribe(() => {
					if (this.R) {
						_.jdb(this.A);
					}
				});
			}
			wn() {
				this.Wa.close();
			}
		};
		q3.J = function(a) {
			return new (a || q3)();
		};
		q3.ka = _.u({
			type: q3,
			da: [["ms-api-key-restrictions-dialog"]],
			ha: 15,
			ia: 2,
			la: () => {
				var a = "· " + _.Sh("{VAR_PLURAL, plural, =1 {1 key} other {{INTERPOLATION} keys}}", {
					INTERPOLATION: "�1�",
					VAR_PLURAL: "�0�"
				});
				var b = "· " + _.Sh("{VAR_PLURAL, plural, =1 {1 key} other {{INTERPOLATION} keys}}", {
					INTERPOLATION: "�1�",
					VAR_PLURAL: "�0�"
				});
				var c = "· " + _.Sh("{VAR_PLURAL, plural, =1 {1 key} other {{INTERPOLATION} keys}}", {
					INTERPOLATION: "�1�",
					VAR_PLURAL: "�0�"
				});
				var d = _.Sh("{VAR_PLURAL, plural, =1 {1 service} other {{INTERPOLATION} services}}", {
					INTERPOLATION: "�1�",
					VAR_PLURAL: "�0�"
				});
				d = _.Sh("[�#1�|�#2�]Gemini API + [�/#1�|�/#2�][�#1�|�#2�] " + d + " [�/#1�|�/#2�]");
				var e = _.Sh("{VAR_PLURAL, plural, =1 {1 service} other {{INTERPOLATION} services}}", {
					INTERPOLATION: "�1�",
					VAR_PLURAL: "�0�"
				});
				return [
					["restrictAction", ""],
					["expandedServices", ""],
					["mixedLabel", ""],
					["otherLabel", ""],
					"Secure your API keys",
					" To improve security, starting on June 19, 2026, the Gemini API will no longer support unrestricted API keys. Update your keys below to ensure your service stays uninterrupted. �#2�Learn more�/#2�",
					"Gemini API usage only",
					a,
					"Restrict keys to Gemini API",
					"Successfully restricted",
					"Some keys failed to restrict",
					"Failed to restrict all API keys",
					"Gemini API only",
					"Mixed usage",
					b,
					" Mixed usage ",
					"Restrict to other services",
					"No Gemini API usage",
					c,
					"Restrict to other services",
					"No usage detected",
					" Other service usage ",
					"Restrict to Gemini API only",
					"Successfully restricted",
					"Failed to restrict",
					d,
					"�#1� " + e + " �/#1�",
					[
						"mat-dialog-title",
						"",
						1,
						"shared-dialog-header"
					],
					[1, "title"],
					[
						"ms-button",
						"",
						"variant",
						"icon-borderless",
						"aria-label",
						"Close dialog",
						3,
						"click",
						"iconName"
					],
					[1, "key-restrictions-container"],
					[1, "loading-spinner-container"],
					["diameter", "40"],
					[1, "dialog-description"],
					["documentation-path", "/gemini-api/docs/api-key#secure-unrestricted-keys"],
					[1, "keys-list"],
					[
						1,
						"group-header",
						"gemini-only-usage"
					],
					[1, "group-title"],
					[
						1,
						"icon",
						3,
						"iconName"
					],
					[
						"matTooltip",
						"The following keys have used only the Gemini API service in the past 90 days.",
						"tabindex",
						"0",
						"aria-label",
						"Information about Gemini API usage",
						1,
						"info-icon",
						3,
						"iconName"
					],
					[1, "key-count"],
					[1, "restrict-all-api-keys-container"],
					[
						"ms-button",
						"",
						1,
						"restrict-all-button",
						3,
						"disabled",
						"matTooltip",
						"ve",
						"veClick",
						"veImpression"
					],
					[1, "key-restricting-spinner"],
					[1, "key-restriction-partial-failed"],
					[1, "key-restriction-failed-all"],
					[1, "key-row"],
					[
						"ms-button",
						"",
						1,
						"restrict-all-button",
						3,
						"click",
						"disabled",
						"matTooltip",
						"ve",
						"veClick",
						"veImpression"
					],
					["diameter", "24"],
					[3, "iconName"],
					[1, "col-key"],
					[3, "apiKey"],
					[1, "key-name"],
					[1, "col-usage"],
					[1, "col-action"],
					[
						4,
						"ngTemplateOutlet",
						"ngTemplateOutletContext"
					],
					[
						1,
						"group-header",
						"mixed-usage"
					],
					[
						"matTooltip",
						"The following keys have used the Gemini API and other services in the past 90 days.",
						"tabindex",
						"0",
						"aria-label",
						"Information about mixed usage",
						1,
						"info-icon",
						3,
						"iconName"
					],
					[
						1,
						"key-name",
						3,
						"matTooltip"
					],
					[1, "usage-title"],
					[
						1,
						"col-action",
						"mixed-usage-action"
					],
					[
						"ms-button",
						"",
						"size",
						"small",
						"documentation-path",
						"/gemini-api/docs/api-key#secure-unrestricted-keys",
						1,
						"restrict-other-services-button",
						3,
						"iconName",
						"isIconPositionEnd",
						"ve",
						"veClick",
						"veImpression"
					],
					[
						1,
						"group-header",
						"no-gemini-usage"
					],
					[
						"matTooltip",
						"The following keys have not used the Gemini API service in the last 90 days.",
						"tabindex",
						"0",
						"aria-label",
						"Information about no Gemini API usage",
						1,
						"info-icon",
						3,
						"iconName"
					],
					[
						"ms-button",
						"",
						"size",
						"small",
						1,
						"restrict-button",
						3,
						"ve",
						"veClick",
						"veImpression"
					],
					[1, "key-restriction-failed"],
					[
						"ms-button",
						"",
						"size",
						"small",
						1,
						"restrict-button",
						3,
						"click",
						"ve",
						"veClick",
						"veImpression"
					],
					[
						"role",
						"button",
						"tabindex",
						"0",
						1,
						"sub-text",
						"clickable",
						3,
						"click",
						"keydown.enter",
						"keydown.space"
					],
					[
						1,
						"expand-icon",
						3,
						"iconName"
					],
					[1, "expanded-services"],
					[1, "service-name"]
				];
			},
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "h2", 27)(1, "div", 28), _.Mh(2, 4), _.H(), _.F(3, "button", 29), _.J("click", function() {
						return b.wn();
					}), _.H()(), _.F(4, "mat-dialog-content", 30), _.B(5, hkd, 2, 0, "div", 31)(6, Gkd, 7, 3), _.H(), _.z(7, Mkd, 4, 1, "ng-template", null, 0, _.Ii)(9, Qkd, 4, 7, "ng-template", null, 1, _.Ii)(11, Rkd, 3, 2, "ng-template", null, 2, _.Ii)(13, Skd, 2, 2, "ng-template", null, 3, _.Ii);
				}
				if (a & 2) {
					_.y(3), _.E("iconName", b.S.ac), _.y(2), _.C(b.ef() || b.EMb() ? 5 : 6);
				}
			},
			dependencies: [
				o3,
				_.Yy,
				_.LC,
				_.dz,
				_.ZB,
				_.xC,
				_.uC,
				_.vC,
				_.$D,
				_.yA,
				_.zC,
				_.yC,
				_.dE,
				_.IC,
				_.HC,
				_.nz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:block;width:100%;max-width:960px;max-height:673px;overflow-y:auto}.header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding:16px 0}.dialog-description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var)}.group-header[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;grid-column:1/-1;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;padding:12px 16px;background-color:var(--color-v3-surface-container-high);margin-top:24px}.group-header.gemini-only-usage[_ngcontent-%COMP%]{-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding-right:0}.group-header.mixed-usage[_ngcontent-%COMP%]{-webkit-box-pack:start;-webkit-justify-content:flex-start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:flex-start}.group-header.no-gemini-usage[_ngcontent-%COMP%]{-webkit-box-pack:start;-webkit-justify-content:flex-start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:flex-start}.group-title[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.restrict-all-api-keys-container[_ngcontent-%COMP%]{width:40%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end}.key-restriction-container[_ngcontent-%COMP%]{width:80%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end}.key-restricting-spinner[_ngcontent-%COMP%]{width:100%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center}.keys-list[_ngcontent-%COMP%]{display:grid;grid-template-columns:33% 1fr -webkit-max-content;grid-template-columns:33% 1fr max-content}.key-row[_ngcontent-%COMP%]{display:grid;grid-template-columns:subgrid;grid-column:1/-1;border-bottom:1px solid var(--color-v3-outline-var);-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start}.key-row[_ngcontent-%COMP%]:last-child{border-bottom:none}.col-action[_ngcontent-%COMP%], .col-key[_ngcontent-%COMP%], .col-usage[_ngcontent-%COMP%]{padding:12px 16px}.col-action[_ngcontent-%COMP%]{padding-right:0}.key-name[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px}.col-usage[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px}.sub-text.clickable[_ngcontent-%COMP%]{cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.expanded-services[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container-high);padding:4px;border-radius:4px;margin-top:4px}.service-name[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-on-surface-variant)}.col-action[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end}.restrict-button[_ngcontent-%COMP%], .restrict-other-services-button[_ngcontent-%COMP%]{width:100%;white-space:nowrap}.mixed-usage-action[_ngcontent-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px}.key-restriction-failed[_ngcontent-%COMP%], .key-restriction-failed-all[_ngcontent-%COMP%], .key-restriction-partial-failed[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.info-icon[_ngcontent-%COMP%]{font-size:16px}.key-count[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.loading-spinner-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;padding:16px}.cloud-console-action-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px}"]
		});
		var jmd = class {
			kda() {
				var a = this;
				return _.x(function* () {
					var b = a.A();
					if (b) {
						a.xDa.set(true);
						try {
							var c = a.H;
							var d = c.kda;
							var e = b.clone();
							var f = a.displayName();
							var g = _.Uc(e, 2, f);
							yield d.call(c, g);
							a.F.show({
								content: "API key renamed successfully!",
								Ne: "success"
							});
						} catch (k) {
							a.F.error("Failed to rename API key.");
						} finally {
							a.Wa.close();
							a.xDa.set(false);
						}
					}
				});
			}
			OFa(a) {
				if (typeof a === "string") {
					this.displayName.set(a), a.length > 63 ? this.wm.set(_.yd()) : this.wm.set("");
				}
			}
			constructor() {
				this.H = _.m(_.Uy);
				this.Wa = _.m(_.kC);
				this.I = _.m(_.qC);
				this.F = _.m(_.iC);
				this.A = _.M();
				this.S = _.Dk;
				this.xDa = _.M(false);
				this.displayName = _.M("");
				this.wm = _.M("");
				this.LIb = _.W(() => !this.displayName() || this.wm().length > 0);
				this.A.set(this.I.key);
				var a;
				var b;
				this.displayName.set((b = (a = this.A()) == null ? undefined : a.getDisplayName()) != null ? b : "");
			}
		};
		jmd.J = function(a) {
			return new (a || jmd)();
		};
		jmd.ka = _.u({
			type: jmd,
			da: [["ms-api-key-update-dialog"]],
			ha: 6,
			ia: 2,
			la: () => [
				"Rename API key",
				" Cancel ",
				" Update ",
				[1, "header"],
				[1, "title"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"matDialogClose",
					"",
					"aria-label",
					"Close dialog",
					3,
					"iconName"
				],
				[1, "spinner-container"],
				[3, "diameter"],
				[1, "display-name-container"],
				[
					"label",
					"Name your key",
					"id",
					"display-name-input",
					"cdkFocusInitial",
					"",
					3,
					"valueChange",
					"value",
					"errorMessage"
				],
				"variant borderless ms-button  matDialogClose ".split(" "),
				[
					"ms-button",
					"",
					"data-test-id",
					"update-button",
					"isIconPositionEnd",
					"",
					3,
					"click",
					"iconName",
					"disabled"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 3)(1, "div", 4), _.Mh(2, 0), _.H(), _.I(3, "button", 5), _.H(), _.B(4, Tkd, 3, 1, "mat-dialog-content")(5, Ukd, 8, 4);
				}
				if (a & 2) {
					_.y(3), _.E("iconName", b.S.ac), _.y(), _.C(b.xDa() ? 4 : 5);
				}
			},
			dependencies: [
				_.Yy,
				_.JD,
				_.mE,
				_.xC,
				_.sC,
				_.wC,
				_.vC,
				_.$D,
				_.zC,
				_.yC,
				_.IC,
				_.MD
			],
			styles: [".header[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;padding:8px}mat-dialog-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;width:100%}.title[_ngcontent-%COMP%]{margin-left:8px;font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px}.description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px}.spinner-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:16px}"]
		});
		var kmd = class {
			constructor() {
				this.apiKey = _.Li.required();
				this.dialog = _.m(_.rC);
			}
		};
		kmd.J = function(a) {
			return new (a || kmd)();
		};
		kmd.ka = _.u({
			type: kmd,
			da: [["ms-blocked-api-key-badge"]],
			inputs: { apiKey: [1, "apiKey"] },
			ha: 3,
			ia: 1,
			la: () => [
				["blockedApiKeyTooltip", ""],
				"Blocked",
				"This key has been blocked from using Gemini API for security purposes. Please create a new API key. ",
				"Learn more",
				" Create new key ",
				[
					"dialogLabel",
					"Blocked API key tooltip",
					"data-test-blocked-badge",
					"",
					1,
					"badge",
					"alert",
					3,
					"xapInlineDialog"
				],
				[1, "blocked-tooltip"],
				["documentation-path", "/gemini-api/docs/api-key#blocked-keys"],
				[
					"ms-button",
					"",
					"size",
					"small",
					"data-test-expand-button",
					"",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Vkd, 2, 1, "span", 5), _.z(1, Wkd, 7, 0, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					a = b.apiKey(), a = _.Z(a, Dkd, 11), _.C((a == null ? 0 : _.Pm(a, 1)) ? 0 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.LC,
				_.IC,
				_.EC
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.badge[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;cursor:pointer}.badge[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.badge.enabled[_ngcontent-%COMP%]:before, .badge.green[_ngcontent-%COMP%]:before, .badge.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.badge.gray[_ngcontent-%COMP%]:before, .badge.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.badge.confidential[_ngcontent-%COMP%]:before, .badge.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.badge.blue[_ngcontent-%COMP%]:before, .badge.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.badge.alert[_ngcontent-%COMP%]:before, .badge.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.badge.hide-circle[_ngcontent-%COMP%]:before{display:none}.blocked-tooltip[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-on-button);padding:8px 12px;width:287px}.blocked-tooltip[_ngcontent-%COMP%]   button[_ngcontent-%COMP%]{margin-top:8px}"]
		});
		var lmd = class {
			constructor() {
				this.ve = { Fsb: 325341 };
				this.apiKey = _.Li.required();
				this.dialog = _.m(_.rC);
			}
			qva() {
				this.dialog.open(q3, { data: { apiKey: this.apiKey() } });
			}
		};
		lmd.J = function(a) {
			return new (a || lmd)();
		};
		lmd.ka = _.u({
			type: lmd,
			da: [["ms-unrestricted-api-key-badge"]],
			inputs: { apiKey: [1, "apiKey"] },
			ha: 3,
			ia: 1,
			la: () => [
				["unrestrictedApiKeyTooltip", ""],
				"Unrestricted",
				"Unrestricted API keys will stop working on June 19, 2026. ",
				" Add restriction ",
				[
					"dialogLabel",
					"Unrestricted API key tooltip",
					"data-test-unrestricted-badge",
					"",
					1,
					"badge",
					"alert",
					3,
					"xapInlineDialog"
				],
				[1, "unrestricted-tooltip"],
				[
					"ms-button",
					"",
					"size",
					"small",
					"data-test-expand-button",
					"",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Xkd, 2, 1, "span", 4), _.z(1, Ykd, 5, 3, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					a = b.apiKey(), a = _.Pm(a, 9), _.C(a ? 0 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.IC,
				_.Bz,
				_.EC
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.badge[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;cursor:pointer}.badge[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.badge.enabled[_ngcontent-%COMP%]:before, .badge.green[_ngcontent-%COMP%]:before, .badge.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.badge.gray[_ngcontent-%COMP%]:before, .badge.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.badge.confidential[_ngcontent-%COMP%]:before, .badge.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.badge.blue[_ngcontent-%COMP%]:before, .badge.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.badge.alert[_ngcontent-%COMP%]:before, .badge.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.badge.hide-circle[_ngcontent-%COMP%]:before{display:none}.unrestricted-tooltip[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-on-button);padding:8px 12px;width:287px}.unrestricted-tooltip[_ngcontent-%COMP%]   button[_ngcontent-%COMP%]{margin-top:8px}"]
		});
		var pmd = class {
			RTb(a, b) {
				return b.getName();
			}
			constructor() {
				this.ve = {
					Gfb: 278849,
					Ifb: 278852,
					Nfb: 278853,
					Ofb: 278850,
					Pfb: 278851
				};
				this.jy = nmd;
				this.S = _.Dk;
				this.nUa = {
					Y9a: 278845,
					uTa: 297230
				};
				this.sort = _.Ni(_.TI);
				this.F = _.m(_.Cl);
				this.dialog = _.m(_.rC);
				this.H = _.m(_.Hu);
				this.C2 = _.m(_.Ou);
				this.R = _.m(_.ZC);
				this.I = _.m(_.Op);
				this.tia = this.I.getFlag(_.xkb);
				this.Kh = this.R.A.Il;
				this.Pm = _.Li([]);
				this.columns = _.Li([]);
				this.Su = _.V();
				this.groupBy = _.V("individual");
				this.Uga = _.M(false);
				this.Gs = new _.fJ();
				this.A = 3;
				this.D9a = _.W(() => omd(this).length - 3);
				this.KOb = _.W(() => this.groupBy() === "by_project" && this.D9a() > 0);
				_.Fk([this.Pm, this.Su], () => {
					if (this.groupBy() === "by_project") {
						this.A = 3;
					} else {
						this.A = omd(this).length;
					}
					this.Gs.data = omd(this).slice(0, this.A);
					this.H.lb();
				});
				_.Fk([this.sort], () => {
					this.Gs.F = this.U;
					var a = this.sort();
					if (a) {
						this.Gs.sort = a;
					}
				});
			}
			MGa(a) {
				this.F.navigate(["/usage"], { queryParams: { project: a.Ya() } });
			}
			kna(a) {
				_.Rn(this.C2, "API", "Clicked Go to billing button on a Project");
				this.F.navigate(["/spend"], { queryParams: { project: a.Ya() } });
			}
			U(a, b) {
				switch (b) {
					case "Created On":
						let c, d;
						return (d = (c = a.aj()) == null ? undefined : c.toDate().getTime()) != null ? d : new Date(0).getTime();
					default: return 0;
				}
			}
			cKa() {
				this.A = this.Uga() ? 3 : omd(this).length;
				this.Gs.data = omd(this).slice(0, this.A);
				this.Uga.set(!this.Uga());
			}
			n6(a) {
				var b = this;
				return _.x(function* () {
					_.Rn(b.C2, "API", "Clicked Delete API Key Button");
					yield _.pf(_.jC(b.dialog.open(_.BC, {
						data: { apiKey: a },
						id: "delete-api-key-dialog"
					})));
				});
			}
		};
		pmd.J = function(a) {
			return new (a || pmd)();
		};
		pmd.ka = _.u({
			type: pmd,
			da: [["ms-api-key-table"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.sort, _.TI, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: {
				Pm: [1, "apiKeys"],
				columns: [1, "columns"],
				Su: [1, "filteredProject"],
				groupBy: [1, "groupBy"]
			},
			ha: 5,
			ia: 2,
			la: () => [
				["actionButtons", ""],
				["overflowmenu", ""],
				"Key",
				"Project",
				"Created",
				"Billing Tier",
				"Project",
				"Project ID",
				"Created",
				"Billing tier",
				"�*3:1� See less �/*3:1��*4:2� See more (�0:2�) �/*4:2�",
				"�#16��/#16� Rename key ",
				"�#20��/#20� Delete key ",
				[1, "api-key-table-container"],
				[1, "api-key-cards-container"],
				[1, "show-container"],
				[
					"mat-table",
					"",
					"matSort",
					"",
					1,
					"mat-elevation-z8",
					3,
					"dataSource",
					"trackBy"
				],
				[3, "matColumnDef"],
				[
					"mat-header-cell",
					"",
					"class",
					"table-header-cell",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell",
					4,
					"matCellDef"
				],
				[
					"mat-header-cell",
					"",
					"class",
					"table-header-cell",
					"mat-sort-header",
					"",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell created-on",
					4,
					"matCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell quota-tier",
					4,
					"matCellDef"
				],
				[
					"mat-header-row",
					"",
					4,
					"matHeaderRowDef"
				],
				[
					"mat-row",
					"",
					4,
					"matRowDef",
					"matRowDefColumns"
				],
				[
					"mat-header-cell",
					"",
					1,
					"table-header-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell"
				],
				[1, "api-key"],
				[3, "apiKey"],
				[1, "sub-text"],
				[3, "compromisedApiKey"],
				[1, "project"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"key-table-link",
					3,
					"click"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"",
					1,
					"table-header-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"created-on"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"quota-tier"
				],
				[
					3,
					"project",
					"veMap"
				],
				[
					4,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				["mat-header-row", ""],
				["mat-row", ""],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"api-key-card"
				],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"api-key-card",
					3,
					"click",
					"keydown.enter"
				],
				[1, "card-header"],
				[1, "card-key-info"],
				[
					1,
					"card-actions",
					3,
					"click",
					"keydown"
				],
				[1, "card-details"],
				[1, "card-detail-row"],
				[1, "card-detail-label"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"key-table-link",
					3,
					"click",
					"keydown"
				],
				[
					1,
					"card-detail-value",
					"sub-text"
				],
				[1, "card-detail-value"],
				[
					1,
					"card-detail-value",
					3,
					"click",
					"keydown"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-expand-button",
					"",
					1,
					"see-more",
					3,
					"click"
				],
				[1, "actions"],
				[
					"matTooltip",
					"Copy API key",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy API key",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata"
				],
				[
					"matTooltip",
					"View spend",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View spend",
					3,
					"click",
					"iconName",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata"
				],
				[
					"matTooltip",
					"View usage",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View usage",
					3,
					"click",
					"iconName",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View more actions",
					"matTooltip",
					"View more actions",
					3,
					"matMenuTriggerFor",
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					"matTooltipPosition",
					"left",
					"data-test-rename-key",
					"",
					3,
					"click",
					"matTooltip",
					"matTooltipDisabled",
					"disabled",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata"
				],
				[
					1,
					"start-icon",
					3,
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					"matTooltipPosition",
					"left",
					"data-test-delete-key",
					"",
					3,
					"click",
					"matTooltip",
					"matTooltipDisabled",
					"disabled",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, qld, 19, 9, "div", 13)(1, yld, 3, 0, "div", 14), _.B(2, Bld, 5, 1, "div", 15), _.z(3, Gld, 21, 44, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.Kh() ? 1 : 0), _.y(2), _.C(b.KOb() ? 2 : -1);
				}
			},
			dependencies: [
				o3,
				kmd,
				_.Yy,
				_.JC,
				_.dz,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.VI,
				_.TI,
				_.UI,
				_.hJ,
				_.gJ,
				_.XI,
				_.aJ,
				_.YI,
				_.WI,
				_.bJ,
				_.ZI,
				_.$I,
				_.cJ,
				_.dJ,
				_.IC,
				_.HC,
				_.nz,
				_.l3,
				lmd,
				_.Bz,
				_.TC,
				_.hz,
				_.pz
			],
			styles: ["[_nghost-%COMP%]   .mat-mdc-row[_ngcontent-%COMP%]   .mat-mdc-cell[_ngcontent-%COMP%]{padding-top:16px;padding-bottom:16px}[_nghost-%COMP%]   .mat-mdc-header-row[_ngcontent-%COMP%]{height:48px}td[_ngcontent-%COMP%]{vertical-align:baseline}.api-key[_ngcontent-%COMP%], .project[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:start;-moz-box-align:start;-ms-flex-align:start;align-items:start;gap:2px}.api-key-table-container[_ngcontent-%COMP%]{overflow-x:auto;width:100%}.key-table-link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;padding:0;margin:0;border:none;height:24px}.table-header-cell[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text-var)}.table-body-cell[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.actions[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text-var);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px;-webkit-box-pack:end;-webkit-justify-content:end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:end}.mat-column-Actions[_ngcontent-%COMP%]{width:12%;vertical-align:middle}.mat-column-Quota-Tier[_ngcontent-%COMP%]{width:12%;min-width:118px}.mat-column-Created-On[_ngcontent-%COMP%]{width:15%;min-width:124px}.mat-column-Key[_ngcontent-%COMP%]{width:20%}.show-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center}.see-more[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var)}ms-compromised-api-key[_ngcontent-%COMP%], ms-unrestricted-api-key-badge[_ngcontent-%COMP%]{margin-top:4px;margin-right:4px}.api-key-cards-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px}.api-key-card[_ngcontent-%COMP%]{border:1px solid var(--color-v3-outline-var);border-radius:12px;padding:16px;background:var(--color-v3-surface-container-high);cursor:pointer;-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.api-key-card.cdk-keyboard-focused[_ngcontent-%COMP%], .api-key-card.cdk-touch-focused[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container-highest)}@media (hover:hover),(pointer:none){.api-key-card[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-surface-container-highest)}}.card-header[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;gap:8px}.card-key-info[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;min-width:0;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}.card-actions[_ngcontent-%COMP%]{-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.card-actions[_ngcontent-%COMP%]   .actions[_ngcontent-%COMP%]{gap:4px}.card-details[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;margin-top:12px;padding-top:12px;border-top:1px solid var(--color-v3-outline-var)}.card-detail-row[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}.card-detail-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.card-detail-value[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text);text-align:end;min-width:0}"]
		});
		var qmd = class {
			constructor() {
				this.ve = {
					Jfb: 278848,
					Kfb: 278843,
					Lfb: 278844
				};
				this.RJ = mmd;
				this.A = _.m(_.ZC);
				this.projects = _.Li([]);
				this.xQ = _.V();
				this.tv = this.A.A.small;
				this.Fja = _.Ki();
				this.Su = _.Ki();
				this.xa = _.M();
				this.xR = _.M("individual");
				this.xh = _.W(() => [{
					value: "all",
					label: "All projects"
				}, ...this.projects().map((a) => ({
					value: a.Ya(),
					label: a.getDisplayName(),
					uca: a.Ya()
				}))]);
				this.ZOb = _.W(() => {
					var a = this.xa();
					return a ? [{
						value: a.Ya(),
						label: a.getDisplayName()
					}] : [{
						value: "all",
						label: "All projects"
					}];
				});
				_.Fk([this.xQ], () => {
					this.xa.set(this.xQ());
				});
			}
			ib() {
				this.Fja.emit(this.xR());
			}
			hGa(a) {
				var b;
				var c = (b = a[0]) == null ? undefined : b.value;
				if (c && c !== "all") {
					if (a = this.projects().find((d) => d.Ya() === c)) {
						this.xa.set(a);
						this.Su.emit(a);
					}
				} else {
					this.xa.set(undefined);
					this.Su.emit(undefined);
				}
			}
		};
		qmd.J = function(a) {
			return new (a || qmd)();
		};
		qmd.ka = _.u({
			type: qmd,
			da: [["ms-api-key-subheader"]],
			inputs: {
				projects: [1, "projects"],
				xQ: [1, "defaultProjectFilter"]
			},
			outputs: {
				Fja: "groupByValue",
				Su: "filteredProject"
			},
			ha: 10,
			ia: 13,
			la: () => [
				" Group by ",
				" API Key ",
				" Project ",
				[1, "group-by-container"],
				[1, "chip-group"],
				[
					"ms-button",
					"",
					"variant",
					"filter-chip",
					"data-test-individual-button",
					"",
					3,
					"click",
					"active",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"filter-chip",
					"data-test-project-button",
					"",
					3,
					"click",
					"active",
					"ve",
					"veClick",
					"veImpression"
				],
				[1, "filter-container"],
				[
					"data-test-id",
					"project-filter-dropdown",
					"labelText",
					"Filter by",
					"noneSelectedText",
					"All projects",
					3,
					"onSelectionChange",
					"options",
					"selected",
					"multiple",
					"showSelectorLabel",
					"ve"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 3)(1, "span"), _.Mh(2, 0), _.H(), _.F(3, "div", 4)(4, "button", 5), _.J("click", function() {
						b.xR.set(b.RJ.rsa);
						b.Fja.emit(b.xR());
					}), _.Mh(5, 1), _.H(), _.F(6, "button", 6), _.J("click", function() {
						b.xR.set(b.RJ.Uda);
						b.Fja.emit(b.xR());
					}), _.Mh(7, 2), _.H()()(), _.F(8, "div", 7)(9, "ms-dashboard-selector", 8), _.J("onSelectionChange", function(c) {
						return b.hGa(c);
					}), _.H()();
				}
				if (a & 2) {
					_.y(4), _.E("active", b.xR() === b.RJ.rsa)("ve", b.ve.Kfb)("veClick", true)("veImpression", true), _.y(2), _.E("active", b.xR() === b.RJ.Uda)("ve", b.ve.Lfb)("veClick", true)("veImpression", true), _.y(3), _.E("options", b.xh())("selected", b.ZOb())("multiple", false)("showSelectorLabel", !b.tv())("ve", b.ve.Jfb);
				}
			},
			dependencies: [
				_.Yy,
				_.eE,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;vertical-align:middle}@media screen and (max-width:600px){[_nghost-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:reverse;-webkit-flex-direction:column-reverse;-moz-box-orient:vertical;-moz-box-direction:reverse;-ms-flex-direction:column-reverse;flex-direction:column-reverse}}.group-by-container[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}@media screen and (max-width:600px){.group-by-container[_ngcontent-%COMP%]{width:100%}}.chip-group[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px}.active[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text)}.filter-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;margin-left:auto}@media screen and (max-width:600px){.filter-container[_ngcontent-%COMP%]{margin-left:0;width:100%}.filter-container[_ngcontent-%COMP%]   ms-dashboard-selector[_ngcontent-%COMP%]{width:100%}}.filter-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var)}"]
		});
		var smd = class {
			constructor() {
				this.ve = { xfb: 325340 };
				this.Xmb = "Learn more";
				this.Pm = _.Li([]);
				this.DZ = _.V(false);
				this.QUa = _.V("");
				this.SB = _.V("");
				this.d4a = _.V("");
				this.o$a = _.V(false);
				this.calloutType = _.V("warning");
				this.wpa = _.M(false);
				this.dialog = _.m(_.rC);
			}
			qva(a) {
				this.dialog.open(q3, { data: { apiKey: a } });
			}
		};
		smd.J = function(a) {
			return new (a || smd)();
		};
		smd.ka = _.u({
			type: smd,
			da: [["ms-api-keys-banner"]],
			inputs: {
				Pm: [1, "apiKeys"],
				DZ: [1, "isDismissable"],
				QUa: [1, "bannerContentText"],
				SB: [1, "learnMoreUrl"],
				d4a: [1, "learnMoreText"],
				o$a: [1, "showAddRestrictionButton"],
				calloutType: [1, "calloutType"]
			},
			ha: 11,
			ia: 7,
			la: () => [
				"Hide API Key(s)",
				"Show API Key(s)",
				" Secure keys ",
				[
					3,
					"calloutType",
					"isDismissable"
				],
				["callout-content-text", ""],
				[1, "api-keys-banner-text"],
				[
					"target",
					"_blank",
					"rel",
					"noopener noreferrer",
					3,
					"href"
				],
				[1, "api-keys-container"],
				["callout-actions", ""],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					1,
					"api-keys-toggle",
					3,
					"click"
				],
				[
					"ms-button",
					"",
					"data-test-add-restriction-button",
					"",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[3, "apiKey"],
				[
					"ms-button",
					"",
					"data-test-add-restriction-button",
					"",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ms-callout", 3)(1, "div", 4)(2, "div", 5), _.R(3), _.B(4, Hld, 2, 2, "a", 6), _.H(), _.B(5, Lld, 3, 0, "div", 7), _.H(), _.F(6, "div", 8)(7, "button", 9), _.J("click", function() {
						b.wpa.set(!b.wpa());
					}), _.B(8, Mld, 2, 0, "ng-container")(9, Nld, 2, 0, "ng-container"), _.H(), _.B(10, Old, 2, 3, "button", 10), _.H()();
				}
				if (a & 2) {
					_.E("calloutType", b.calloutType())("isDismissable", b.DZ()), _.y(3), _.S(" ", b.QUa(), " "), _.y(), _.C(b.SB() ? 4 : -1), _.y(), _.C(b.wpa() ? 5 : -1), _.y(3), _.C(b.wpa() ? 8 : 9), _.y(2), _.C(b.o$a() && b.Pm().length > 0 ? 10 : -1);
				}
			},
			dependencies: [
				o3,
				_.Yy,
				_.zA,
				_.tz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:block}ms-callout[_ngcontent-%COMP%]{margin-bottom:32px}[callout-actions][_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}"]
		});
		var tmd = class {
			constructor() {
				this.S = _.Dk;
				this.ve = {
					Hfb: 278847,
					Mfb: 278846
				};
				this.dialog = _.m(_.rC);
				this.ePa = _.m(_.Ou);
				this.Gq = _.M(false);
				this.H = _.m(_.ZC);
				this.F = _.m(_.$E);
				this.A = this.F.get("projectFilter");
				this.tv = this.H.A.small;
			}
			U$() {
				var a = this;
				return _.x(function* () {
					_.Rn(a.ePa, "API", "Clicked Create API Key Button");
					var b = a.A();
					a.Gq.set(true);
					var c = { II: true };
					if (b) {
						c.fH = b;
					}
					b = yield _.pf(_.jC(a.dialog.open(_.AE, { data: c })));
					a.Gq.set(false);
					return b;
				});
			}
		};
		tmd.J = function(a) {
			return new (a || tmd)();
		};
		tmd.ka = _.u({
			type: tmd,
			da: [["ms-api-keys-header"]],
			ha: 9,
			ia: 11,
			la: () => [
				"API Keys",
				"�*6:1� API quickstart �/*6:1�",
				" Create API key ",
				[1, "header-container"],
				[1, "header"],
				[1, "right-side"],
				[
					"ms-button",
					"",
					"href",
					"https://ai.google.dev/gemini-api/docs/quickstart",
					"target",
					"_blank",
					"data-test-id",
					"api-quickstart-button",
					"aria-label",
					"API quickstart",
					1,
					"api-quickstart-link",
					3,
					"click",
					"variant",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"data-test-id",
					"create-api-key-button",
					3,
					"click",
					"iconName",
					"disabled",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 3)(1, "h2", 4), _.Mh(2, 0), _.H(), _.F(3, "div", 5)(4, "a", 6), _.J("click", function() {
						_.Rn(b.ePa, "API", "Clicked API Quickstart Button");
					}), _.Kh(5, 1), _.B(6, Pld, 1, 0), _.Lh(), _.H(), _.F(7, "button", 7), _.J("click", function() {
						return b.U$();
					}), _.Mh(8, 2), _.H()()();
				}
				if (a & 2) {
					_.y(4), _.E("variant", b.tv() ? "icon-borderless" : "borderless")("iconName", b.S.DOCS)("ve", b.ve.Mfb)("veClick", true)("veImpression", true), _.y(2), _.C(b.tv() ? -1 : 6), _.y(), _.E("iconName", b.tv() ? undefined : b.S.ly)("disabled", b.Gq())("ve", b.ve.Hfb)("veClick", true)("veImpression", true);
				}
			},
			dependencies: [
				_.Yy,
				_.xC,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{width:100%;margin-bottom:32px}.header[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:24px;font-weight:600;line-height:32px}.header-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}@media screen and (max-width:768px){[_nghost-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start}[_nghost-%COMP%] > .header-container[_ngcontent-%COMP%]{-webkit-box-ordinal-group:1;-webkit-order:0;-moz-box-ordinal-group:1;-ms-flex-order:0;order:0;width:100%}}.right-side[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-flow:row wrap;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-flow:row wrap;flex-flow:row wrap;gap:20px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}@media screen and (max-width:600px){.right-side[_ngcontent-%COMP%]{gap:4px}}.api-quickstart-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}"]
		});
		_.vs = class {
			constructor() {
				this.JPa = "Can't find your API keys here?";
				this.IPa = "This list only shows API keys for projects imported into Google AI Studio. Import other projects to manage their associated API Keys. You can also create a new API Key above.";
				this.RJ = mmd;
				this.FNa = rmd;
				this.A = _.m(_.Uy);
				this.Za = _.m(_.Iy);
				this.I = _.m(_.sG);
				this.fPa = _.m(_.$E);
				_.m(_.Cl);
				this.H = _.m(_.GG);
				this.R = _.m(_.Op);
				this.projects = this.Za.Sd;
				this.groupBy = _.M("individual");
				this.Su = _.M();
				this.fEa = _.W(() => this.A.ef() || this.I.I());
				this.VGb = this.R.getFlag(_.xkb);
				this.F = this.fPa.get("projectFilter");
				this.Pm = this.A.A;
				this.SMb = _.W(() => {
					var a = new Map();
					for (let b of this.Pm()) {
						let c;
						let d = (c = b.te()) == null ? undefined : c.Ya();
						if (!d) continue;
						let e;
						if (!(this.Su() && ((e = this.Su()) == null ? undefined : e.Ya()) !== d)) {
							a.has(d) ? a.get(d).push(b) : a.set(d, [b]);
						}
					}
					return a;
				});
				this.Ncb = _.W(() => this.Pm().filter((a) => _.Pm(a, 9)));
				this.SWa = _.W(() => this.Pm().filter((a) => {
					var b;
					return (b = _.Qn(a)) == null ? undefined : _.Pm(b, 1);
				}));
				this.columns = _.W(() => this.groupBy() === "by_project" ? [
					"Key",
					"Created On",
					"Quota Tier",
					"Actions"
				] : [
					"Key",
					"Project",
					"Created On",
					"Quota Tier",
					"Actions"
				]);
				try {
					_.Sy(this.A);
				} catch (a) {
					console.error("Failed to fetch API keys:", a);
				}
				_.Fk([this.F, this.projects], () => {
					if (this.F()) {
						let a = this.projects().find((b) => b.Ya() === this.F());
						this.Su.set(a);
					}
				});
				_.FG(this.H);
			}
		};
		_.vs.J = function(a) {
			return new (a || _.vs)();
		};
		_.vs.ka = _.u({
			type: _.vs,
			da: [["ms-api-keys"]],
			ha: 10,
			ia: 5,
			la: () => [
				[1, "page-content-wrapper"],
				[1, "page-content-inner-wrapper"],
				[
					"bannerContentText",
					"Unrestricted API keys will stop working on June 19, 2026. Add restrictions to your keys below to avoid service interruption.",
					"learnMoreUrl",
					"https://ai.google.dev/gemini-api/docs/api-key#secure-unrestricted-keys",
					3,
					"apiKeys",
					"showAddRestrictionButton",
					"calloutType"
				],
				[
					"bannerContentText",
					"We detected a publicly exposed API key.",
					"learnMoreUrl",
					"https://ai.google.dev/gemini-api/docs/api-key#security",
					"learnMoreText",
					"Learn how to secure your API keys",
					3,
					"apiKeys",
					"calloutType",
					"isDismissable"
				],
				[
					3,
					"groupByValue",
					"filteredProject",
					"projects",
					"defaultProjectFilter"
				],
				[1, "loading-spinner"],
				[3, "diameter"],
				[
					3,
					"columns",
					"apiKeys",
					"filteredProject"
				],
				[
					"learnMoreUrl",
					"https://ai.google.dev/gemini-api/docs/api-key#import-projects",
					3,
					"headline",
					"message",
					"showSparkle",
					"showImportProjectsButton",
					"showCreateProjectButton"
				],
				[1, "project-header"],
				[1, "project-name"],
				[1, "project-id"],
				[
					3,
					"columns",
					"apiKeys",
					"groupBy"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "div", 1), _.I(2, "ms-api-keys-header")(3, "ms-payment-alert-callout"), _.B(4, Qld, 1, 3, "ms-api-keys-banner", 2), _.B(5, Rld, 1, 3, "ms-api-keys-banner", 3), _.F(6, "ms-api-key-subheader", 4), _.J("groupByValue", function(c) {
						b.groupBy.set(c);
					})("filteredProject", function(c) {
						b.Su.set(c);
						b.fPa.set({ projectFilter: c == null ? undefined : c.Ya() });
					}), _.H(), _.B(7, Sld, 2, 1, "div", 5)(8, Tld, 2, 8)(9, Xld, 3, 5), _.H()();
				}
				if (a & 2) {
					_.y(4), _.C(b.VGb && b.Ncb().length > 0 ? 4 : -1), _.y(), _.C(b.SWa().length > 0 ? 5 : -1), _.y(), _.E("projects", b.projects())("defaultProjectFilter", b.Su()), _.y(), _.C(b.fEa() ? 7 : b.groupBy() === b.RJ.rsa ? 8 : b.groupBy() === b.RJ.Uda ? 9 : -1);
				}
			},
			dependencies: [
				qmd,
				tmd,
				pmd,
				smd,
				_.tz,
				_.zC,
				_.yC,
				_.m3,
				_.n3,
				_.tA
			],
			styles: ["[_nghost-%COMP%]{display:block}[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:min(1400px,90%)}@media screen and (max-width:600px){[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:100%}}.project-header[_ngcontent-%COMP%]{margin-top:40px;margin-bottom:16px}.project-id[_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.project-name[_ngcontent-%COMP%]{color:var(--color-v3-text)}.loading-spinner[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:16px}ms-api-key-subheader[_ngcontent-%COMP%]{margin-bottom:24px}.no-keys-container[_ngcontent-%COMP%]{margin:15px auto 45px;width:400px;text-align:center}.no-keys-container[_ngcontent-%COMP%]   img[_ngcontent-%COMP%]{margin-top:60px;margin-bottom:32px}.no-keys-container[_ngcontent-%COMP%]   .no-keys-button[_ngcontent-%COMP%]{margin:32px auto 36px}.no-keys-container[_ngcontent-%COMP%]   .only-one-key-button[_ngcontent-%COMP%]{margin:16px auto 32px}.no-keys-text[_ngcontent-%COMP%]{margin-top:32px;text-align:center}.no-keys-text[_ngcontent-%COMP%]   .no-keys-text-header[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;margin-bottom:8px;color:var(--color-v3-text)}.no-keys-text[_ngcontent-%COMP%]   .no-keys-text-body[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.no-keys-text[_ngcontent-%COMP%]   .no-keys-text-body[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{color:var(--color-v3-text-link);display:block}ms-callout[_ngcontent-%COMP%]{margin-bottom:32px}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

