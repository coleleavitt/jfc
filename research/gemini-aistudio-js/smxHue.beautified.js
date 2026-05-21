"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		nle = function(a) {
			if (a & 1) {
				_.F(0, "a", 4), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.E("href", _.wi(a.Gh.href), _.rg), _.y(), _.U(a.Gh.label);
			}
		};
		ole = function(a) {
			if (a & 1) {
				_.F(0, "a", 5), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.E("routerLink", a.Gh.routerLink.path)("queryParams", a.UL(a.Gh.routerLink.queryParams)), _.y(), _.U(a.Gh.label);
			}
		};
		ple = function(a) {
			if (a & 1) {
				_.F(0, "a", 11), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("href", _.wi(a.href), _.rg), _.y(), _.U(a.label);
			}
		};
		qle = function(a) {
			if (a & 1) {
				_.F(0, "a", 12), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("documentation-path", _.wi(a.zn)), _.y(), _.U(a.label);
			}
		};
		rle = function(a) {
			if (a & 1) {
				_.F(0, "a", 13), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V;
				let b = _.K(2);
				_.E("routerLink", a.routerLink.path)("queryParams", b.UL(a.routerLink.queryParams));
				_.y();
				_.U(a.label);
			}
		};
		sle = function(a, b) {
			if (a & 1) {
				_.B(0, ple, 2, 3, "a", 11)(1, qle, 2, 3, "a", 12)(2, rle, 2, 3, "a", 13);
			}
			if (a & 2) {
				a = b.V, _.C(a.href ? 0 : a.zn ? 1 : a.routerLink ? 2 : -1);
			}
		};
		ule = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 6)(1, "h4", 10), _.R(2), _.H(), _.Ah(3, sle, 3, 1, null, null, tle), _.H();
			}
			if (a & 2) {
				a = b.V, _.y(2), _.U(a.title), _.y(), _.Bh(a.links);
			}
		};
		vle = function(a) {
			return Object.fromEntries(a);
		};
		wle = (a, b) => b.title;
		tle = (a, b) => b.label;
		_.h8 = class {
			constructor() {
				this.theme = _.V("dark");
				this.description = "Start exploring and building with Google’s latest models.";
				this.Gh = {
					label: "Sign up and get started",
					routerLink: {
						path: "/apps",
						queryParams: []
					}
				};
				this.columns = [
					{
						title: "Models",
						links: [
							{
								label: "Gemini",
								zn: "/gemini-api/docs/models/gemini-3.1-pro-preview"
							},
							{
								label: "Nano Banana",
								routerLink: {
									path: "/models/gemini-3-1-flash-image",
									queryParams: []
								}
							},
							{
								label: "Veo",
								routerLink: {
									path: "/models/veo-3",
									queryParams: []
								}
							}
						]
					},
					{
						title: "Product",
						links: [
							{
								label: "AI Studio",
								routerLink: {
									path: "/apps",
									queryParams: []
								}
							},
							{
								label: "App gallery",
								routerLink: {
									path: "/apps",
									queryParams: [
										["source", "showcase"],
										["NLb", "featured"],
										["showcaseTag", "featured"]
									]
								}
							},
							{
								label: "Documentation",
								zn: "/gemini-api/docs"
							},
							{
								label: "Gemini API key",
								zn: "/gemini-api/docs/models/gemini-3.1-pro-preview"
							},
							{
								label: "Pricing",
								zn: "/gemini-api/docs/pricing"
							},
							{
								label: "Case studies",
								routerLink: {
									path: "/case-studies",
									queryParams: []
								}
							}
						]
					},
					{
						title: "Capabilities",
						links: [{
							label: "Vibe coding",
							routerLink: {
								path: "/vibe-code",
								queryParams: []
							}
						}]
					},
					{
						title: "Information",
						links: [
							{
								label: "Gemini app",
								href: "https://gemini.google/about/"
							},
							{
								label: "Privacy",
								href: "https://policies.google.com/privacy"
							},
							{
								label: "Terms",
								href: "https://ai.google.dev/gemini-api/terms"
							}
						]
					}
				];
				this.UL = vle;
			}
		};
		_.h8.J = function(a) {
			return new (a || _.h8)();
		};
		_.h8.ka = _.u({
			type: _.h8,
			da: [["ms-marketing-footer"]],
			Ua: 2,
			Ja: function(a, b) {
				if (a & 2) {
					_.P("theme-light", b.theme() === "light");
				}
			},
			inputs: { theme: [1, "theme"] },
			ha: 12,
			ia: 2,
			la: [
				[1, "footer"],
				[1, "footer_divider"],
				[1, "footer__brand"],
				[1, "footer__description"],
				[
					1,
					"footer__cta",
					"btn",
					"btn--primary",
					3,
					"href"
				],
				[
					1,
					"footer__cta",
					"btn",
					"btn--primary",
					3,
					"routerLink",
					"queryParams"
				],
				[1, "footer__col"],
				[1, "footer__bottom"],
				[1, "footer__logo-large"],
				[
					"src",
					"https://www.gstatic.com/aistudio-static/vibe_code/footer_logo.svg",
					"alt",
					"Google AI Studio large logo",
					1,
					"footer__logo-img"
				],
				[1, "footer__col-title"],
				[
					1,
					"footer__col-link",
					3,
					"href"
				],
				[
					1,
					"footer__col-link",
					3,
					"documentation-path"
				],
				[
					1,
					"footer__col-link",
					3,
					"routerLink",
					"queryParams"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "footer", 0), _.I(1, "div", 1), _.F(2, "div", 2)(3, "p", 3), _.R(4), _.H(), _.B(5, nle, 2, 3, "a", 4)(6, ole, 2, 3, "a", 5), _.H(), _.Ah(7, ule, 5, 1, "div", 6, wle), _.F(9, "div", 7)(10, "div", 8), _.I(11, "img", 9), _.H()()();
				}
				if (a & 2) {
					_.y(4), _.U(b.description), _.y(), _.C(b.Gh.href ? 5 : b.Gh.routerLink ? 6 : -1), _.y(2), _.Bh(b.columns);
				}
			},
			dependencies: [_.sA, _.LC],
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*[_ngcontent-%COMP%], [_ngcontent-%COMP%]:after, [_ngcontent-%COMP%]:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}[_nghost-%COMP%]{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff;background-color:#121317}img[_ngcontent-%COMP%], video[_ngcontent-%COMP%]{display:block;max-width:100%;height:auto}a[_ngcontent-%COMP%]{color:inherit;text-decoration:none}button[_ngcontent-%COMP%]{font-family:inherit;cursor:pointer;border:none;background:none}ol[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]{list-style:none}input[_ngcontent-%COMP%], textarea[_ngcontent-%COMP%]{font-family:inherit;border:none;outline:none;background:none}h1[_ngcontent-%COMP%], h2[_ngcontent-%COMP%], h3[_ngcontent-%COMP%], h4[_ngcontent-%COMP%], h5[_ngcontent-%COMP%], h6[_ngcontent-%COMP%]{font-weight:400;line-height:1.2}.text-muted[_ngcontent-%COMP%]{color:#b2bbc5}.text-accent[_ngcontent-%COMP%]{color:#2e96ff}.material-symbols-outlined[_ngcontent-%COMP%]{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}.btn[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;border-radius:12px;padding:8px 16px;-webkit-transition:background-color .2s ease;transition:background-color .2s ease;white-space:nowrap;cursor:pointer;text-decoration:none}.btn--primary[_ngcontent-%COMP%]{background-color:#fff;color:#0e0f12}.btn--primary[_ngcontent-%COMP%]:hover{background-color:#e6eaf0}.btn--primary[_ngcontent-%COMP%]:active{background-color:#cdd4dc}.btn--primary.is-disabled[_ngcontent-%COMP%], .btn--primary[_ngcontent-%COMP%]:disabled{background-color:#18191d;color:#45474d;cursor:default}.btn--large[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;padding:14px 24px 14px 20px}.btn--large[_ngcontent-%COMP%]   .btn__icon[_ngcontent-%COMP%]{font-size:20px}.btn__icon[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-family:Material Symbols Outlined;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon-svg[_ngcontent-%COMP%]{width:14px;height:14px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.footer[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;width:100%;max-width:1540px;margin-left:auto;margin-right:auto;gap:20px;padding-left:20px;padding-right:20px;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;padding-bottom:20px;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start}@media (min-width:1024px){.footer[_ngcontent-%COMP%]{max-width:1580px;padding-left:40px;padding-right:40px}}@media (min-width:1024px){.footer[_ngcontent-%COMP%]{padding-bottom:40px}}.footer_divider[_ngcontent-%COMP%]{width:100%;height:1px;background-color:transparent;margin-bottom:40px}@media (min-width:1024px){.footer_divider[_ngcontent-%COMP%]{margin-bottom:80px}}.footer__brand[_ngcontent-%COMP%]{width:100%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;margin-bottom:32px}@media (min-width:1024px){.footer__brand[_ngcontent-%COMP%]{width:calc(33.3333333333% - 13.3333333333px);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;margin-bottom:0}}.footer__description[_ngcontent-%COMP%]{color:#fff;font-size:24px;line-height:25.92px;letter-spacing:-.144px;margin-bottom:24px;max-width:400px}@media (min-width:1024px){.footer__description[_ngcontent-%COMP%]{max-width:329px}}.footer__cta[_ngcontent-%COMP%]{margin-top:0}.footer__col[_ngcontent-%COMP%]{width:calc(50% - 10px);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;margin-bottom:24px}@media (min-width:1024px){.footer__col[_ngcontent-%COMP%]{width:calc(16.6666666667% - 16.6666666667px);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;margin-bottom:0}}.footer__col-title[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;color:#fff;letter-spacing:.1925px;margin-bottom:10px}.footer__col-link[_ngcontent-%COMP%]{font-size:14.5px;line-height:11px;color:#b2bbc5;letter-spacing:.11px;-webkit-transition:color .2s ease;transition:color .2s ease}.footer__col-link[_ngcontent-%COMP%] + .footer__col-link[_ngcontent-%COMP%]{margin-top:20px}.footer__col-link[_ngcontent-%COMP%]:hover{color:#fff}.footer__bottom[_ngcontent-%COMP%]{width:100%;margin-top:40px}@media (min-width:1024px){.footer__bottom[_ngcontent-%COMP%]{margin-top:80px}}.footer__logo-large[_ngcontent-%COMP%]{width:100%}.footer__logo-large[_ngcontent-%COMP%]   .footer__logo-img[_ngcontent-%COMP%]{width:100%;height:auto;display:block}.theme-light[_nghost-%COMP%]{display:block;width:100%;background:#f8f9fa}.theme-light[_nghost-%COMP%]   .footer_divider[_ngcontent-%COMP%]{background-color:transparent}.theme-light[_nghost-%COMP%]   .footer__description[_ngcontent-%COMP%]{color:#1f1f1f}.theme-light[_nghost-%COMP%]   .footer__cta[_ngcontent-%COMP%]{background:#1f1f1f;color:#fff}.theme-light[_nghost-%COMP%]   .footer__cta[_ngcontent-%COMP%]:hover{background:#333}.theme-light[_nghost-%COMP%]   .footer__col-title[_ngcontent-%COMP%]{color:#1f1f1f}.theme-light[_nghost-%COMP%]   .footer__col-link[_ngcontent-%COMP%]{color:#5f6368}.theme-light[_nghost-%COMP%]   .footer__col-link[_ngcontent-%COMP%]:hover{color:#1f1f1f}.theme-light[_nghost-%COMP%]   .footer__logo-img[_ngcontent-%COMP%]{-webkit-filter:invert(1);filter:invert(1)}"]
		});
		xle = function(a) {
			if (a & 1) {
				_.F(0, "a", 10), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("href", _.wi(a.href), _.rg), _.y(), _.U(a.label);
			}
		};
		yle = function(a) {
			if (a & 1) {
				_.F(0, "a", 11), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("documentation-path", _.wi(a.zn)), _.y(), _.U(a.label);
			}
		};
		zle = function(a) {
			if (a & 1) {
				_.F(0, "a", 12), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("routerLink", _.wi(a.routerLink)), _.y(), _.U(a.label);
			}
		};
		Ale = function(a, b) {
			if (a & 1) {
				_.B(0, xle, 2, 3, "a", 10)(1, yle, 2, 3, "a", 11)(2, zle, 2, 3, "a", 12);
			}
			if (a & 2) {
				a = b.V, _.C(a.href ? 0 : a.zn ? 1 : a.routerLink ? 2 : -1);
			}
		};
		Ble = function(a) {
			if (a & 1) {
				_.F(0, "span", 15), _.I(1, "span", 5), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.Gh().icon);
			}
		};
		Cle = function(a) {
			if (a & 1) {
				_.F(0, "a", 13), _.B(1, Ble, 2, 1, "span", 15), _.R(2), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("href", _.wi(a.Gh().href), _.rg), _.y(), _.C(a.Gh().icon ? 1 : -1), _.y(), _.S(" ", a.Gh().label, " ");
			}
		};
		Dle = function(a) {
			if (a & 1) {
				_.F(0, "span", 15), _.I(1, "span", 5), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.Gh().icon);
			}
		};
		Ele = function(a) {
			if (a & 1) {
				_.F(0, "a", 14), _.B(1, Dle, 2, 1, "span", 15), _.R(2), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("routerLink", _.wi(a.Gh().routerLink)), _.y(), _.C(a.Gh().icon ? 1 : -1), _.y(), _.S(" ", a.Gh().label, " ");
			}
		};
		Fle = function(a) {
			if (a & 1) {
				_.B(0, Cle, 3, 4, "a", 13)(1, Ele, 3, 4, "a", 14);
			}
			if (a & 2) {
				a = _.K(), _.C(a.Gh().href ? 0 : a.Gh().routerLink ? 1 : -1);
			}
		};
		Gle = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 19);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.mw());
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("href", _.wi(a.href), _.rg), _.y(), _.U(a.label);
			}
		};
		Hle = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 20);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.mw());
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("documentation-path", _.wi(a.zn)), _.y(), _.U(a.label);
			}
		};
		Ile = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 21);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.mw());
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("routerLink", _.wi(a.routerLink)), _.y(), _.U(a.label);
			}
		};
		Jle = function(a, b) {
			if (a & 1) {
				_.B(0, Gle, 2, 3, "a", 16)(1, Hle, 2, 3, "a", 17)(2, Ile, 2, 3, "a", 18);
			}
			if (a & 2) {
				a = b.V, _.C(a.href ? 0 : a.zn ? 1 : a.routerLink ? 2 : -1);
			}
		};
		Kle = function(a) {
			if (a & 1) {
				_.F(0, "span", 15), _.I(1, "span", 5), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.Gh().icon);
			}
		};
		Lle = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 24);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.mw());
				});
				_.B(1, Kle, 2, 1, "span", 15);
				_.R(2);
				_.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("href", _.wi(a.Gh().href), _.rg), _.y(), _.C(a.Gh().icon ? 1 : -1), _.y(), _.S(" ", a.Gh().label, " ");
			}
		};
		Mle = function(a) {
			if (a & 1) {
				_.F(0, "span", 15), _.I(1, "span", 5), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("iconName", a.Gh().icon);
			}
		};
		Nle = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 25);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.mw());
				});
				_.B(1, Mle, 2, 1, "span", 15);
				_.R(2);
				_.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("routerLink", _.wi(a.Gh().routerLink)), _.y(), _.C(a.Gh().icon ? 1 : -1), _.y(), _.S(" ", a.Gh().label, " ");
			}
		};
		Ole = function(a) {
			if (a & 1) {
				_.B(0, Lle, 3, 4, "a", 22)(1, Nle, 3, 4, "a", 23);
			}
			if (a & 2) {
				a = _.K(), _.C(a.Gh().href ? 0 : a.Gh().routerLink ? 1 : -1);
			}
		};
		Ple = (a, b) => b.label;
		_.i8 = class {
			constructor() {
				this.qf = _.Dk;
				this.bx = false;
				this.links = [
					{
						label: "Pricing",
						zn: "/gemini-api/docs/pricing"
					},
					{
						label: "Documentation",
						zn: "/gemini-api/docs"
					},
					{
						label: "Case studies",
						routerLink: "/case-studies"
					}
				];
				this.Gh = _.Li({
					label: "Get started",
					routerLink: "/",
					icon: "login"
				});
			}
			Tca() {
				this.bx = !this.bx;
			}
			mw() {
				this.bx = false;
			}
		};
		_.i8.J = function(a) {
			return new (a || _.i8)();
		};
		_.i8.ka = _.u({
			type: _.i8,
			da: [["ms-marketing-nav"]],
			inputs: { Gh: [1, "cta"] },
			ha: 17,
			ia: 7,
			la: [
				[1, "nav"],
				[
					"routerLink",
					"/welcome",
					1,
					"nav__logo",
					3,
					"click"
				],
				[
					"src",
					"https://www.gstatic.com/aistudio-static/mobile/ais-logo.svg",
					"alt",
					"Google AI Studio",
					1,
					"nav__logo-img"
				],
				[1, "nav__links"],
				[
					1,
					"nav__menu-btn",
					3,
					"click"
				],
				[3, "iconName"],
				[1, "nav__mobile-overlay"],
				[1, "nav__mobile-links"],
				[1, "nav__mobile-footer"],
				[1, "nav__mobile-text"],
				[
					1,
					"nav__link",
					3,
					"href"
				],
				[
					1,
					"nav__link",
					3,
					"documentation-path"
				],
				[
					1,
					"nav__link",
					3,
					"routerLink"
				],
				[
					1,
					"nav__cta",
					"btn",
					"btn--primary",
					3,
					"href"
				],
				[
					1,
					"nav__cta",
					"btn",
					"btn--primary",
					3,
					"routerLink"
				],
				[1, "btn__icon"],
				[
					1,
					"nav__mobile-link",
					3,
					"href"
				],
				[
					1,
					"nav__mobile-link",
					3,
					"documentation-path"
				],
				[
					1,
					"nav__mobile-link",
					3,
					"routerLink"
				],
				[
					1,
					"nav__mobile-link",
					3,
					"click",
					"href"
				],
				[
					1,
					"nav__mobile-link",
					3,
					"click",
					"documentation-path"
				],
				[
					1,
					"nav__mobile-link",
					3,
					"click",
					"routerLink"
				],
				[
					1,
					"nav__mobile-cta",
					"btn",
					"btn--primary",
					3,
					"href"
				],
				[
					1,
					"nav__mobile-cta",
					"btn",
					"btn--primary",
					3,
					"routerLink"
				],
				[
					1,
					"nav__mobile-cta",
					"btn",
					"btn--primary",
					3,
					"click",
					"href"
				],
				[
					1,
					"nav__mobile-cta",
					"btn",
					"btn--primary",
					3,
					"click",
					"routerLink"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "nav", 0)(1, "a", 1), _.J("click", function() {
						return b.mw();
					}), _.I(2, "img", 2), _.H(), _.F(3, "div", 3), _.Ah(4, Ale, 3, 1, null, null, Ple), _.H(), _.B(6, Fle, 2, 1), _.F(7, "button", 4), _.J("click", function() {
						return b.Tca();
					}), _.I(8, "span", 5), _.H(), _.F(9, "div", 6)(10, "div", 7), _.Ah(11, Jle, 3, 1, null, null, Ple), _.H(), _.F(13, "div", 8)(14, "p", 9), _.R(15, "Start exploring and building with Google latest models."), _.H(), _.B(16, Ole, 2, 1), _.H()()();
				}
				if (a & 2) {
					_.P("nav--menu-open", b.bx), _.y(4), _.Bh(b.links), _.y(2), _.C(b.Gh() ? 6 : -1), _.y(2), _.E("iconName", b.bx ? b.qf.ac : b.qf.L2), _.y(), _.P("nav__mobile-overlay--open", b.bx), _.y(2), _.Bh(b.links), _.y(5), _.C(b.Gh() ? 16 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.LC,
				_.sA,
				_.dz
			],
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*[_ngcontent-%COMP%], [_ngcontent-%COMP%]:after, [_ngcontent-%COMP%]:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}[_nghost-%COMP%]{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff;background-color:#121317}img[_ngcontent-%COMP%], video[_ngcontent-%COMP%]{display:block;max-width:100%;height:auto}a[_ngcontent-%COMP%]{color:inherit;text-decoration:none}button[_ngcontent-%COMP%]{font-family:inherit;cursor:pointer;border:none;background:none}ol[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]{list-style:none}input[_ngcontent-%COMP%], textarea[_ngcontent-%COMP%]{font-family:inherit;border:none;outline:none;background:none}h1[_ngcontent-%COMP%], h2[_ngcontent-%COMP%], h3[_ngcontent-%COMP%], h4[_ngcontent-%COMP%], h5[_ngcontent-%COMP%], h6[_ngcontent-%COMP%]{font-weight:400;line-height:1.2}.text-muted[_ngcontent-%COMP%]{color:#b2bbc5}.text-accent[_ngcontent-%COMP%]{color:#2e96ff}.material-symbols-outlined[_ngcontent-%COMP%]{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}.btn[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;border-radius:12px;padding:8px 16px;-webkit-transition:background-color .2s ease;transition:background-color .2s ease;white-space:nowrap;cursor:pointer;text-decoration:none}.btn--primary[_ngcontent-%COMP%]{background-color:#fff;color:#0e0f12}.btn--primary[_ngcontent-%COMP%]:hover{background-color:#e6eaf0}.btn--primary[_ngcontent-%COMP%]:active{background-color:#cdd4dc}.btn--primary.is-disabled[_ngcontent-%COMP%], .btn--primary[_ngcontent-%COMP%]:disabled{background-color:#18191d;color:#45474d;cursor:default}.btn--large[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;padding:14px 24px 14px 20px}.btn--large[_ngcontent-%COMP%]   .btn__icon[_ngcontent-%COMP%]{font-size:20px}.btn__icon[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-family:Material Symbols Outlined;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon-svg[_ngcontent-%COMP%]{width:14px;height:14px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.nav[_ngcontent-%COMP%]{position:fixed;top:0;left:0;width:100%;z-index:100;padding:22px 20px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background-color:rgba(18,19,23,.7);backdrop-filter:blur(20px) saturate(180%);-webkit-backdrop-filter:blur(20px) saturate(180%)}@media (min-width:1024px){.nav[_ngcontent-%COMP%]{padding:22px 40px}}.nav__logo[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:2px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;position:relative;z-index:2}.nav__logo-img[_ngcontent-%COMP%]{height:24px;width:auto}@media (min-width:1024px){.nav__logo-img[_ngcontent-%COMP%]{height:28px}}.nav__links[_ngcontent-%COMP%]{display:none;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:31px;margin-left:40px}@media (min-width:1024px){.nav__links[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}}.nav__link[_ngcontent-%COMP%]{font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;color:#cdd4dc;-webkit-transition:color .2s ease;transition:color .2s ease;text-decoration:none}.nav__link[_ngcontent-%COMP%]:hover{color:#fff}.nav__cta[_ngcontent-%COMP%]{display:none;margin-left:auto}@media (min-width:1024px){.nav__cta[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex}}.nav__menu-btn[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;margin-left:auto;font-family:Material Symbols Outlined;font-size:24px;color:#fff;cursor:pointer;position:relative;z-index:2}@media (min-width:1024px){.nav__menu-btn[_ngcontent-%COMP%]{display:none}}.nav__mobile-overlay[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;position:fixed;top:0;left:0;width:100vw;height:100vh;background-color:#0e0f12;z-index:1;padding:80px 20px 40px;opacity:0;visibility:hidden;-webkit-transition:opacity .3s ease,visibility .3s ease;transition:opacity .3s ease,visibility .3s ease}.nav__mobile-overlay--open[_ngcontent-%COMP%]{opacity:1;visibility:visible}.dark-theme[_nghost-%COMP%]   .nav__mobile-overlay[_ngcontent-%COMP%], .dark-theme   [_nghost-%COMP%]   .nav__mobile-overlay[_ngcontent-%COMP%]{background-color:#0e0f12}@media (min-width:1024px){.nav__mobile-overlay[_ngcontent-%COMP%]{display:none}}.nav__mobile-links[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;margin-bottom:auto}.nav__mobile-link[_ngcontent-%COMP%]{font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:32px;line-height:39px;letter-spacing:-.5px;color:#fff;text-decoration:none;margin-bottom:24px;-webkit-transition:color .2s ease;transition:color .2s ease}.nav__mobile-link[_ngcontent-%COMP%]:last-child{margin-bottom:0}.nav__mobile-link[_ngcontent-%COMP%]:hover{color:#d4d4d4}.nav__mobile-footer[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;margin-top:auto}.nav__mobile-text[_ngcontent-%COMP%]{font-family:Google Sans Flex,sans-serif;font-size:22px;line-height:24px;letter-spacing:-.13px;max-width:256px;color:#cdd4dc;margin:0 0 24px 0}"]
		});
		var ite;
		var jte;
		Use = function(a) {
			if (a & 1) {
				_.I(0, "span", 9);
			}
			if (a & 2) {
				a = _.K().V, _.E("iconName", a.icon);
			}
		};
		Vse = function(a) {
			if (a & 1) {
				_.I(0, "img", 10);
			}
			if (a & 2) {
				a = _.K().V, _.E("src", a.icon, _.rg);
			}
		};
		Wse = function(a) {
			if (a & 1) {
				_.F(0, "span", 14), _.I(1, "span", 15), _.H();
			}
			if (a & 2) {
				a = _.K(2).V, _.y(), _.E("iconName", a.ctaIcon);
			}
		};
		Xse = function(a) {
			if (a & 1) {
				_.F(0, "a", 13), _.B(1, Wse, 2, 1, "span", 14), _.R(2), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("href", a.Ju, _.rg), _.y(), _.C(a.ctaIcon ? 1 : -1), _.y(), _.S(" ", a.Iu, " ");
			}
		};
		Yse = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 7)(1, "div", 8), _.B(2, Use, 1, 1, "span", 9)(3, Vse, 1, 1, "img", 10), _.F(4, "h3", 11), _.R(5), _.H()(), _.F(6, "p", 12), _.R(7), _.H(), _.B(8, Xse, 3, 3, "a", 13), _.H();
			}
			if (a & 2) {
				a = b.V;
				b = b.jb;
				let c = _.K();
				_.P("is-active", c.iva === b);
				_.wh("data-block", b);
				_.y(2);
				_.C(a.Vk === "font" ? 2 : a.Vk === "svg-file" ? 3 : -1);
				_.y(3);
				_.U(a.title);
				_.y(2);
				_.U(a.description);
				_.y();
				_.C(a.Iu && a.Ju ? 8 : -1);
			}
		};
		Zse = function(a) {
			if (a & 1) {
				_.I(0, "video", 17, 1);
			}
			if (a & 2) {
				a = _.K().V, _.E("src", a.video, _.rg);
			}
		};
		$se = function(a) {
			if (a & 1) {
				_.I(0, "img", 18);
			}
			if (a & 2) {
				a = _.K().V, _.E("src", a.image, _.rg);
			}
		};
		ate = function(a) {
			if (a & 1) {
				_.I(0, "span", 9);
			}
			if (a & 2) {
				a = _.K().V, _.E("iconName", a.icon);
			}
		};
		bte = function(a) {
			if (a & 1) {
				_.I(0, "img", 10);
			}
			if (a & 2) {
				a = _.K().V, _.E("src", a.icon, _.rg);
			}
		};
		cte = function(a) {
			if (a & 1) {
				_.F(0, "span", 14), _.I(1, "span", 15), _.H();
			}
			if (a & 2) {
				a = _.K(2).V, _.y(), _.E("iconName", a.ctaIcon);
			}
		};
		dte = function(a) {
			if (a & 1) {
				_.F(0, "a", 13), _.B(1, cte, 2, 1, "span", 14), _.R(2), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.E("href", a.Ju, _.rg), _.y(), _.C(a.ctaIcon ? 1 : -1), _.y(), _.S(" ", a.Iu, " ");
			}
		};
		ete = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 6)(1, "div", 16, 0), _.B(3, Zse, 2, 1, "video", 17)(4, $se, 1, 1, "img", 18), _.H(), _.F(5, "div", 19)(6, "div", 8), _.B(7, ate, 1, 1, "span", 9)(8, bte, 1, 1, "img", 10), _.F(9, "h3", 11), _.R(10), _.H()(), _.F(11, "p", 12), _.R(12), _.H(), _.B(13, dte, 3, 3, "a", 13), _.H()();
			}
			if (a & 2) {
				a = b.V, b = b.jb, _.y(), _.wh("data-block", b), _.y(2), _.C(a.video ? 3 : a.image ? 4 : -1), _.y(4), _.C(a.Vk === "font" ? 7 : a.Vk === "svg-file" ? 8 : -1), _.y(3), _.U(a.title), _.y(2), _.U(a.description), _.y(), _.C(a.Iu && a.Ju ? 13 : -1);
			}
		};
		fte = ["mediaItem"];
		gte = ["video"];
		ite = function(a) {
			if (a.observer) {
				a.observer.disconnect();
			}
			a.A.clear();
			a.observer = new IntersectionObserver((b) => {
				hte(a, b);
			}, {
				root: null,
				rootMargin: "-30% 0px -30% 0px",
				threshold: 0
			});
			a.T4a().forEach((b) => {
				a.observer.observe(b.nativeElement);
			});
		};
		jte = function(a) {
			a.Bg().forEach((b) => {
				b = b.nativeElement;
				b.defaultMuted = true;
				b.muted = true;
				b.play().catch(() => {});
			});
		};
		hte = function(a, b) {
			b.forEach((c) => {
				var d = c.target.dataset.block;
				if (d !== undefined) {
					c.isIntersecting ? a.A.add(d) : a.A.delete(d);
				}
			});
			b = kte(a);
			if (b !== undefined) {
				a.iva = Number(b);
			} else {
				a.iva = null;
			}
			a.ti.lb();
		};
		kte = function(a) {
			if (a.A.size !== 0) return [...a.A].sort((b, c) => Number(b) - Number(c))[0];
		};
		_.r8 = class {
			constructor() {
				this.S = _.Dk;
				this.blocks = _.Li([]);
				this.T4a = _.hi();
				this.Bg = _.hi();
				this.iva = null;
				this.ti = _.m(_.Hu);
				this.A = new Set();
				_.Kg(() => {
					ite(this);
				});
				_.Kg(() => {
					jte(this);
				});
			}
			qz() {
				if (this.F) {
					clearTimeout(this.F);
				}
				this.F = setTimeout(() => {
					ite(this);
				}, 100);
			}
			Ba() {
				if (this.F) {
					clearTimeout(this.F);
				}
				if (this.observer) {
					this.observer.disconnect();
				}
			}
		};
		_.r8.J = function(a) {
			return new (a || _.r8)();
		};
		_.r8.ka = _.u({
			type: _.r8,
			da: [["ms-marketing-blocks"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.T4a, fte, 5)(b.Bg, gte, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			Ja: function(a, b) {
				if (a & 1) {
					_.J("resize", function() {
						return b.qz();
					}, _.Te);
				}
			},
			inputs: { blocks: [1, "blocks"] },
			ha: 7,
			ia: 0,
			la: [
				["mediaItem", ""],
				["video", ""],
				[1, "blocks"],
				[1, "blocks__text-col"],
				[
					1,
					"blocks__text-item",
					3,
					"is-active"
				],
				[1, "blocks__media-col"],
				[1, "blocks__group"],
				[1, "blocks__text-item"],
				[1, "blocks__text-title-row"],
				[
					1,
					"blocks__text-icon",
					3,
					"iconName"
				],
				[
					"alt",
					"",
					1,
					"blocks__text-icon-svg",
					3,
					"src"
				],
				[1, "blocks__text-title"],
				[1, "blocks__text-description"],
				[
					"target",
					"_blank",
					1,
					"blocks__text-cta",
					"btn",
					"btn--primary",
					3,
					"href"
				],
				[1, "btn__icon"],
				[3, "iconName"],
				[1, "blocks__media-item"],
				[
					"muted",
					"",
					"autoplay",
					"",
					"loop",
					"",
					"playsinline",
					"",
					3,
					"src"
				],
				[
					"alt",
					"",
					3,
					"src"
				],
				[
					1,
					"blocks__text-item",
					"blocks__text-item--mobile"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "section", 2)(1, "div", 3), _.Ah(2, Yse, 9, 7, "div", 4, _.yh), _.H(), _.F(4, "div", 5), _.Ah(5, ete, 14, 6, "div", 6, _.yh), _.H()();
				}
				if (a & 2) {
					_.y(2), _.Bh(b.blocks()), _.y(3), _.Bh(b.blocks());
				}
			},
			dependencies: [_.dz],
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*[_ngcontent-%COMP%], [_ngcontent-%COMP%]:after, [_ngcontent-%COMP%]:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}[_nghost-%COMP%]{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff;background-color:#121317}img[_ngcontent-%COMP%], video[_ngcontent-%COMP%]{display:block;max-width:100%;height:auto}a[_ngcontent-%COMP%]{color:inherit;text-decoration:none}button[_ngcontent-%COMP%]{font-family:inherit;cursor:pointer;border:none;background:none}ol[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]{list-style:none}input[_ngcontent-%COMP%], textarea[_ngcontent-%COMP%]{font-family:inherit;border:none;outline:none;background:none}h1[_ngcontent-%COMP%], h2[_ngcontent-%COMP%], h3[_ngcontent-%COMP%], h4[_ngcontent-%COMP%], h5[_ngcontent-%COMP%], h6[_ngcontent-%COMP%]{font-weight:400;line-height:1.2}.text-muted[_ngcontent-%COMP%]{color:#b2bbc5}.text-accent[_ngcontent-%COMP%]{color:#2e96ff}.material-symbols-outlined[_ngcontent-%COMP%]{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}.btn[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;border-radius:12px;padding:8px 16px;-webkit-transition:background-color .2s ease;transition:background-color .2s ease;white-space:nowrap;cursor:pointer;text-decoration:none}.btn--primary[_ngcontent-%COMP%]{background-color:#fff;color:#0e0f12}.btn--primary[_ngcontent-%COMP%]:hover{background-color:#e6eaf0}.btn--primary[_ngcontent-%COMP%]:active{background-color:#cdd4dc}.btn--primary.is-disabled[_ngcontent-%COMP%], .btn--primary[_ngcontent-%COMP%]:disabled{background-color:#18191d;color:#45474d;cursor:default}.btn--large[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;padding:14px 24px 14px 20px}.btn--large[_ngcontent-%COMP%]   .btn__icon[_ngcontent-%COMP%]{font-size:20px}.btn__icon[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-family:Material Symbols Outlined;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon-svg[_ngcontent-%COMP%]{width:14px;height:14px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.blocks[_ngcontent-%COMP%]{position:relative;background:#121317;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;width:100%;max-width:1540px;margin-left:auto;margin-right:auto;gap:20px;padding-left:20px;padding-right:20px;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;padding-top:40px;padding-bottom:40px}@media (min-width:1024px){.blocks[_ngcontent-%COMP%]{max-width:1580px;padding-left:40px;padding-right:40px}}@media (min-width:1024px){.blocks[_ngcontent-%COMP%]{-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;padding-top:80px;padding-bottom:80px}}.blocks__text-col[_ngcontent-%COMP%]{display:none}@media (min-width:1024px){.blocks__text-col[_ngcontent-%COMP%]{display:block;width:calc(33.3333333333% - 13.3333333333px);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;position:-webkit-sticky;position:sticky;top:120px;-webkit-align-self:flex-start;-ms-flex-item-align:start;align-self:flex-start;height:auto;margin-bottom:0}}.blocks__media-col[_ngcontent-%COMP%]{width:100%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:52px}@media (min-width:1024px){.blocks__media-col[_ngcontent-%COMP%]{width:calc(50% - 10px);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;margin-left:auto;gap:80px}}.blocks__text-item[_ngcontent-%COMP%]{-webkit-transition:opacity .3s ease,margin-bottom .3s ease;transition:opacity .3s ease,margin-bottom .3s ease;margin-bottom:24px}@media (min-width:1024px){.blocks__text-item[_ngcontent-%COMP%]{margin-left:40px}}.blocks__text-item.is-active[_ngcontent-%COMP%]{margin-bottom:48px}.blocks__text-item[_ngcontent-%COMP%]:last-child{margin-bottom:0}.blocks__text-icon[_ngcontent-%COMP%]{font-size:32px;color:#2e96ff;line-height:1;position:absolute;right:calc(100% + 12px);top:0;opacity:0;-webkit-transition:opacity .3s ease;transition:opacity .3s ease}.is-active[_ngcontent-%COMP%]   .blocks__text-icon[_ngcontent-%COMP%]{opacity:1}.blocks__text-icon-svg[_ngcontent-%COMP%]{position:absolute;right:calc(100% + 12px);top:0;width:24px;height:auto;opacity:0;-webkit-transition:opacity .3s ease;transition:opacity .3s ease}@media (min-width:1250px){.blocks__text-icon-svg[_ngcontent-%COMP%]{width:28px}}.is-active[_ngcontent-%COMP%]   .blocks__text-icon-svg[_ngcontent-%COMP%]{opacity:1}.blocks__text-title[_ngcontent-%COMP%]{font-size:24px;line-height:1.15;letter-spacing:-.4px;color:#fff;display:inline}@media (min-width:1250px){.blocks__text-title[_ngcontent-%COMP%]{font-size:32px;line-height:1.06;letter-spacing:-.512px}}.blocks__text-item[_ngcontent-%COMP%]:not(.is-active)   .blocks__text-title[_ngcontent-%COMP%]{color:#45474d}.blocks__text-title-row[_ngcontent-%COMP%]{position:relative;margin-bottom:8px}.blocks__text-description[_ngcontent-%COMP%]{font-size:14.5px;line-height:1.45;letter-spacing:.11px;margin-bottom:0;color:#b2bbc5;max-width:368px;opacity:0;max-height:0;overflow:hidden;-webkit-transition:opacity .3s ease,max-height .3s ease,margin-bottom .3s ease;transition:opacity .3s ease,max-height .3s ease,margin-bottom .3s ease}@media (min-width:1250px){.blocks__text-description[_ngcontent-%COMP%]{font-size:17.5px;letter-spacing:.1925px}}.is-active[_ngcontent-%COMP%]   .blocks__text-description[_ngcontent-%COMP%]{opacity:1;max-height:150px;margin-bottom:20px}@media (min-width:1024px){.is-active[_ngcontent-%COMP%]   .blocks__text-description[_ngcontent-%COMP%]{margin-bottom:42px}}.blocks__text-cta[_ngcontent-%COMP%]{opacity:0;max-height:0;overflow:hidden;margin-bottom:0;-webkit-transition:opacity .3s ease,max-height .3s ease,margin-bottom .3s ease;transition:opacity .3s ease,max-height .3s ease,margin-bottom .3s ease}.is-active[_ngcontent-%COMP%]   .blocks__text-cta[_ngcontent-%COMP%]{opacity:1;max-height:100px}.blocks__media-item[_ngcontent-%COMP%]{border-radius:20px;overflow:hidden;aspect-ratio:670/670}@media (min-width:1024px){.blocks__media-item[_ngcontent-%COMP%]{border-radius:36px}}.blocks__media-item[_ngcontent-%COMP%]   img[_ngcontent-%COMP%], .blocks__media-item[_ngcontent-%COMP%]   video[_ngcontent-%COMP%]{width:100%;height:100%;object-fit:cover}.blocks__group[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.blocks__text-item--mobile[_ngcontent-%COMP%]{margin-top:24px;margin-bottom:0}@media (min-width:1024px){.blocks__text-item--mobile[_ngcontent-%COMP%]{display:none}}.blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-title-row[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;margin-bottom:12px}.blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-icon[_ngcontent-%COMP%], .blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-icon-svg[_ngcontent-%COMP%]{position:static;-webkit-transform:none;transform:none;opacity:1;margin-right:12px}.blocks__text-item--mobile.blocks__text-item[_ngcontent-%COMP%]   .blocks__text-title[_ngcontent-%COMP%]{color:#fff}.blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-cta[_ngcontent-%COMP%], .blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-description[_ngcontent-%COMP%]{opacity:1;max-height:none}.blocks__text-item--mobile[_ngcontent-%COMP%]   .blocks__text-description[_ngcontent-%COMP%]{margin-bottom:20px}"]
		});
		rte = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "video", 16, 0);
				_.J("loadeddata", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.tT());
				});
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.P("is-loaded", a.aG());
			}
		};
		ste = function(a) {
			if (a & 1) {
				_.F(0, "h2", 6), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.U(a.rEa());
			}
		};
		tte = function(a) {
			if (a & 1) {
				_.F(0, "h3", 19), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.U(a.title());
			}
		};
		ute = function(a) {
			if (a & 1) {
				_.F(0, "p", 20), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.U(a.description());
			}
		};
		vte = function(a) {
			if (a & 1) {
				_.F(0, "div", 17), _.B(1, tte, 2, 1, "h3", 19), _.B(2, ute, 2, 1, "p", 20), _.F(3, "div", 21), _.I(4, "span", 22), _.R(5, " 4 min "), _.H()();
			}
			if (a & 2) {
				a = _.K(2), _.y(), _.C(a.title() ? 1 : -1), _.y(), _.C(a.description() ? 2 : -1), _.y(2), _.E("iconName", a.S.uQa);
			}
		};
		wte = function(a) {
			if (a & 1) {
				_.F(0, "a", 23)(1, "span", 25), _.I(2, "span", 22), _.H(), _.R(3), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.E("href", a.primaryCtaUrl(), _.rg), _.y(2), _.E("iconName", a.S.ly), _.y(), _.S(" ", a.primaryCtaText(), " ");
			}
		};
		xte = function(a) {
			if (a & 1) {
				_.F(0, "a", 24)(1, "span", 25), _.I(2, "span", 22), _.H(), _.R(3), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.E("href", a.secondaryCtaUrl(), _.rg), _.y(2), _.E("iconName", a.S.BPa), _.y(), _.S(" ", a.secondaryCtaText(), " ");
			}
		};
		yte = function(a) {
			if (a & 1) {
				_.F(0, "div", 18), _.B(1, wte, 4, 3, "a", 23), _.B(2, xte, 4, 3, "a", 24), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.y(), _.C(a.primaryCtaText() && a.primaryCtaUrl() ? 1 : -1), _.y(), _.C(a.secondaryCtaText() && a.secondaryCtaUrl() ? 2 : -1);
			}
		};
		zte = function(a) {
			if (a & 1) {
				_.F(0, "div", 8), _.B(1, vte, 6, 3, "div", 17), _.B(2, yte, 3, 2, "div", 18), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.C(a.title() || a.description() ? 1 : -1), _.y(), _.C(a.primaryCtaText() || a.secondaryCtaText() ? 2 : -1);
			}
		};
		Ate = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 27);
				_.J("click", function() {
					var d = _.q(c).V;
					_.K(2).kq.set(d.id);
					return _.t();
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = b.V, b = _.K(2), _.P("is-active", b.kq() === a.id), _.y(), _.S(" ", a.label, " ");
			}
		};
		Cte = function(a) {
			if (a & 1) {
				_.F(0, "div", 10), _.Ah(1, Ate, 2, 3, "button", 26, Bte), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.Bh(a.tabs());
			}
		};
		Dte = function(a, b) {
			if (a & 1) {
				_.F(0, "span"), _.R(1), _.H();
			}
			if (a & 2) {
				a = b.V, _.y(), _.U(a);
			}
		};
		Ete = ["video"];
		Bte = (a, b) => b.id;
		Fte = function(a) {
			return a.split("\n").map((b, c) => c + 1);
		};
		_.t8 = class {
			constructor() {
				this.S = _.Dk;
				this.ho = _.Ni("video");
				this.rEa = _.V();
				this.title = _.V();
				this.description = _.V();
				this.primaryCtaText = _.V();
				this.primaryCtaUrl = _.V();
				this.secondaryCtaText = _.V();
				this.secondaryCtaUrl = _.V();
				this.Bda = _.M(false);
				this.aG = _.M(false);
				this.tabs = _.Li([]);
				this.uab = _.Li({});
				this.kq = _.M();
				this.Ga = _.m(_.Jf);
				this.Qlb = _.m(_.ZL);
				this.Dya = _.V(false);
				_.Fk([this.ho, this.Dya], () => {
					var a = this.ho();
					if (a && !this.Dya()) {
						a.nativeElement.defaultMuted = true, a.nativeElement.muted = true, a.nativeElement.play().catch(() => {});
					}
				});
			}
			Rb() {
				this.A = new IntersectionObserver((a) => {
					if (a[0].isIntersecting) {
						this.Bda.set(true);
						let b;
						if (!((b = this.A) == null)) {
							b.disconnect();
						}
					}
				});
				this.A.observe(this.Ga.nativeElement);
			}
			Ba() {
				var a;
				if (!((a = this.A) == null)) {
					a.disconnect();
				}
			}
			tT() {
				this.aG.set(true);
			}
			ib() {
				var a = this.kq();
				var b = this.tabs();
				if (!(!(b.length > 0) || a && b.find((c) => c.id === a))) {
					this.kq.set(b[0].id);
				}
			}
		};
		_.t8.J = function(a) {
			return new (a || _.t8)();
		};
		_.t8.ka = _.u({
			type: _.t8,
			da: [["ms-marketing-code-snippet"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.ho, Ete, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: {
				rEa: [1, "mainTitle"],
				title: [1, "title"],
				description: [1, "description"],
				primaryCtaText: [1, "primaryCtaText"],
				primaryCtaUrl: [1, "primaryCtaUrl"],
				secondaryCtaText: [1, "secondaryCtaText"],
				secondaryCtaUrl: [1, "secondaryCtaUrl"],
				tabs: [1, "tabs"],
				uab: [1, "snippets"],
				Dya: [1, "disableVideo"]
			},
			ha: 17,
			ia: 5,
			la: [
				["video", ""],
				[1, "code-snippet"],
				[1, "code-snippet__media"],
				[1, "code-snippet__bg"],
				[
					"src",
					"https://www.gstatic.com/aistudio-static/vibe_code/prompt_builder_particles.mp4",
					"autoplay",
					"",
					"muted",
					"",
					"loop",
					"",
					"playsinline",
					"",
					3,
					"is-loaded"
				],
				[1, "code-snippet__content"],
				[1, "code-snippet__main-title"],
				[1, "code-snippet__box"],
				[1, "code-snippet__text"],
				[1, "code-snippet__viewer"],
				[1, "code-snippet__tabs"],
				[1, "code-snippet__code-container"],
				[1, "code-snippet__code-wrapper"],
				[1, "code-snippet__line-numbers"],
				[1, "code-snippet__code"],
				[3, "innerHTML"],
				[
					"src",
					"https://www.gstatic.com/aistudio-static/vibe_code/prompt_builder_particles.mp4",
					"autoplay",
					"",
					"muted",
					"",
					"loop",
					"",
					"playsinline",
					"",
					3,
					"loadeddata"
				],
				[1, "code-snippet__text-top"],
				[1, "code-snippet__ctas"],
				[1, "code-snippet__title"],
				[1, "code-snippet__description"],
				[1, "code-snippet__timer"],
				[3, "iconName"],
				[
					1,
					"btn",
					"btn--primary",
					3,
					"href"
				],
				[
					1,
					"btn",
					"btn--secondary",
					3,
					"href"
				],
				[1, "btn__icon"],
				[
					1,
					"code-snippet__tab",
					3,
					"is-active"
				],
				[
					1,
					"code-snippet__tab",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "section", 1)(1, "div", 2)(2, "div", 3), _.B(3, rte, 2, 2, "video", 4), _.H(), _.F(4, "div", 5), _.B(5, ste, 2, 1, "h2", 6), _.F(6, "div", 7), _.B(7, zte, 3, 2, "div", 8), _.F(8, "div", 9), _.B(9, Cte, 3, 0, "div", 10), _.F(10, "div", 11)(11, "div", 12)(12, "div", 13), _.Ah(13, Dte, 2, 1, "span", null, _.zh), _.H(), _.F(15, "pre", 14), _.I(16, "code", 15), _.H()()()()()()()();
				}
				if (a & 2) {
					_.y(3);
					_.C(b.Bda() ? 3 : -1);
					_.y(2);
					_.C(b.rEa() ? 5 : -1);
					_.y(2);
					_.C(b.title() || b.description() || b.primaryCtaText() || b.secondaryCtaText() ? 7 : -1);
					_.y(2);
					_.C(b.tabs().length > 0 ? 9 : -1);
					{
						a = b.kq();
						let c = b.uab();
						if (a && c[a]) {
							a = c[a], b = b.Qlb.highlight(a.code, a.lang).value, b = {
								mX: _.tdb.Em(b),
								lineNumbers: Fte(a.code)
							};
						} else {
							b = {
								mX: "",
								lineNumbers: []
							};
						}
					}
					_.y(4);
					_.Bh(b.lineNumbers);
					_.y(3);
					_.E("innerHTML", b.mX, _.qg);
				}
			},
			dependencies: [_.tz, _.dz],
			styles: [":root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*,:after,:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}:host{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff;background-color:#121317}img,video{display:block;max-width:100%;height:auto}a{color:inherit;text-decoration:none}button{font-family:inherit;cursor:pointer;border:none;background:none}ol,ul{list-style:none}input,textarea{font-family:inherit;border:none;outline:none;background:none}h1,h2,h3,h4,h5,h6{font-weight:400;line-height:1.2}.text-muted{color:#b2bbc5}.text-accent{color:#2e96ff}.material-symbols-outlined{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}.btn{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;border-radius:12px;padding:8px 16px;-webkit-transition:background-color .2s ease;transition:background-color .2s ease;white-space:nowrap;cursor:pointer;text-decoration:none}.btn--primary{background-color:#fff;color:#0e0f12}.btn--primary:hover{background-color:#e6eaf0}.btn--primary:active{background-color:#cdd4dc}.btn--primary.is-disabled,.btn--primary:disabled{background-color:#18191d;color:#45474d;cursor:default}.btn--large{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;padding:14px 24px 14px 20px}.btn--large .btn__icon{font-size:20px}.btn__icon{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-family:Material Symbols Outlined;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon .material-symbols-outlined{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon-svg{width:14px;height:14px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.hljs{display:block;overflow-x:auto;padding:.5em;background:#f0f0f0}.hljs,.hljs-subst{color:#444}.hljs-comment{color:#888}.hljs-attribute,.hljs-doctag,.hljs-keyword,.hljs-meta-keyword,.hljs-name,.hljs-selector-tag{font-weight:700}.hljs-deletion,.hljs-number,.hljs-quote,.hljs-selector-class,.hljs-selector-id,.hljs-string,.hljs-template-tag,.hljs-type{color:#800}.hljs-section,.hljs-title{color:#800;font-weight:700}.hljs-link,.hljs-regexp,.hljs-selector-attr,.hljs-selector-pseudo,.hljs-symbol,.hljs-template-variable,.hljs-variable{color:#bc6060}.hljs-literal{color:#78a960}.hljs-addition,.hljs-built_in,.hljs-bullet,.hljs-code{color:#397300}.hljs-meta{color:#1f7199}.hljs-meta-string{color:#4d99bf}.hljs-emphasis{font-style:italic}.hljs-strong{font-weight:700}.code-snippet{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;width:100%;max-width:1540px;margin-left:auto;margin-right:auto;gap:20px;padding-left:20px;padding-right:20px;padding-top:40px;padding-bottom:40px}@media (min-width:1024px){.code-snippet{max-width:1580px;padding-left:40px;padding-right:40px}}@media (min-width:1024px){.code-snippet{padding-top:80px;padding-bottom:80px}}.code-snippet__media{width:100%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;position:relative;min-height:400px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;overflow:hidden;border-radius:20px}@media (min-width:1024px){.code-snippet__media{min-height:600px;border-radius:36px}}.code-snippet__bg{position:absolute;inset:0;z-index:0;mix-blend-mode:lighten}.code-snippet__bg video{width:100%;height:100%;object-fit:cover;opacity:0;-webkit-transition:opacity 1s ease-in-out;transition:opacity 1s ease-in-out}.code-snippet__bg video.is-loaded{opacity:1}.code-snippet__content{position:relative;z-index:2;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;width:100%}.code-snippet__main-title{font-size:32px;line-height:1.1;letter-spacing:-.5px;color:#fff;margin-bottom:32px;text-align:center}@media (min-width:1024px){.code-snippet__main-title{font-size:54px;line-height:1.04;letter-spacing:-1.08px;margin-bottom:48px}}.code-snippet__box{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;width:100%;max-width:1128px;background:#18191d;border-radius:16px;border:1px solid #212226;padding:16px;padding-bottom:24px;gap:32px}@media (min-width:1024px){.code-snippet__box{-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:64px;padding:24px}}.code-snippet__text{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}@media (min-width:1024px){.code-snippet__text{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;max-width:240px}}.code-snippet__text-top{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.code-snippet__title{font-size:20px;line-height:1.4;color:#fff;margin-bottom:12px;font-weight:500}.code-snippet__description{font-size:14.5px;line-height:1.6;color:#b2bbc5;margin-bottom:12px}.code-snippet__timer{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:6px;color:#b2bbc5;font-size:14.5px;margin-bottom:32px}.code-snippet__timer .material-symbols-outlined{font-size:18px}.code-snippet__ctas{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;gap:8px}.code-snippet__ctas a{color:#0e0f12}.code-snippet__ctas .btn--secondary{background:#323232;border-color:#323232;color:#fff}.code-snippet__ctas .btn--secondary:hover{background:#424242;border-color:#424242}.code-snippet__viewer{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;background:#0e0f12;border-radius:16px;overflow:hidden;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;min-width:0}.code-snippet__tabs{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;background:#0e0f12;border-bottom:1px solid hsla(0,0%,77%,.08);padding:0 16px}.code-snippet__tab{position:relative;padding:16px 24px;color:#787d8b;font-size:14.5px;font-weight:500;cursor:pointer;border-bottom:2px solid transparent;-webkit-transition:all .2s;transition:all .2s;background:none;border:none}.code-snippet__tab:hover{color:#fff}.code-snippet__tab.is-active{color:#217bfe;border-bottom-color:transparent}.code-snippet__tab.is-active:after{content:\"\";position:absolute;bottom:-1px;left:0;width:100%;height:4px;background-color:#217bfe;border-radius:2px}.code-snippet__code-container{padding:24px 24px 32px 16px;overflow-x:auto;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}@media (min-width:1024px){.code-snippet__code-container{padding:24px}}.code-snippet__code-wrapper{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;font-family:monospace;font-size:14.5px;line-height:1.6;color:#e8eaed}.code-snippet__line-numbers{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;color:#5c6370;padding-right:16px;text-align:right;min-width:30px;-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none}.code-snippet__code{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;margin:0;white-space:pre-wrap}.code-snippet__code .hljs-keyword{color:#81e1ff}.code-snippet__code .hljs-string{color:#d87bc5}.code-snippet__code .hljs-comment{color:#5c6370}.code-snippet__code .hljs-function{color:#217bfe}.code-snippet__code .hljs-title{color:#217bfe}.code-snippet__code .hljs-number{color:#d19a66}.code-snippet__code .hljs-class{color:#e5c07b}.code-snippet__code .hljs-variable{color:#e06c75}"],
			Ab: 2
		});
		var Gte;
		var Hte;
		var Ite;
		var Mte;
		var Nte;
		Gte = function(a) {
			if (a & 1) {
				_.I(0, "img", 3);
			}
			if (a & 2) {
				a = _.K(), _.E("src", a.QS(), _.rg);
			}
		};
		Hte = function(a) {
			if (a & 1) {
				_.I(0, "video", 4, 0);
			}
			if (a & 2) {
				a = _.K(), _.E("src", a.QS(), _.rg);
			}
		};
		Ite = function(a) {
			if (a & 1) {
				_.F(0, "h1", 6), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.U(a.title());
			}
		};
		Jte = function(a) {
			if (a & 1) {
				_.I(0, "span", 12);
			}
			if (a & 2) {
				a = _.K().V, _.E("iconName", a.icon);
			}
		};
		Kte = function(a, b) {
			if (a & 1) {
				_.F(0, "a", 11), _.B(1, Jte, 1, 1, "span", 12), _.R(2), _.H();
			}
			if (a & 2) {
				a = b.V, _.P("btn--primary", a.variant === "primary")("btn--secondary", a.variant === "secondary"), _.E("href", a.url, _.rg), _.y(), _.C(a.icon ? 1 : -1), _.y(), _.S(" ", a.text, " ");
			}
		};
		Mte = function(a) {
			if (a & 1) {
				_.Ah(0, Kte, 3, 7, "a", 10, Lte);
			}
			if (a & 2) {
				a = _.K(), _.Bh(a.YA());
			}
		};
		Nte = function(a) {
			if (a & 1) {
				_.F(0, "a", 9), _.Ee(), _.F(1, "svg", 13), _.I(2, "path", 14), _.H(), _.R(3), _.H();
			}
			if (a & 2) {
				a = _.K(), _.E("href", a.Ju(), _.rg), _.y(3), _.S(" ", a.Iu(), " ");
			}
		};
		Ote = ["video"];
		Lte = (a, b) => b.url;
		_.u8 = class {
			constructor() {
				this.S = _.Dk;
				this.title = _.V();
				this.layout = _.V("default");
				this.YA = _.Li([]);
				this.mediaType = _.V("image");
				this.QS = _.V("assets/images/hero-bg.jpg");
				this.subtitle = _.V("Turn your ideas into real apps with Gemini in Google AI Studio.");
				this.Ju = _.V("https://aistudio.google.com/apps");
				this.Iu = _.V("Build your first app");
			}
			set ho(a) {
				if (this.mediaType() === "video" && a) {
					a.nativeElement.defaultMuted = true, a.nativeElement.muted = true, a.nativeElement.play().catch(() => {});
				}
			}
		};
		_.u8.J = function(a) {
			return new (a || _.u8)();
		};
		_.u8.ka = _.u({
			type: _.u8,
			da: [["ms-marketing-hero"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ci(Ote, 5);
				}
				if (a & 2) {
					let c;
					if (_.ei(c = _.fi())) {
						b.ho = c.first;
					}
				}
			},
			inputs: {
				title: [1, "title"],
				layout: [1, "layout"],
				YA: [1, "ctas"],
				mediaType: [1, "mediaType"],
				QS: [1, "mediaSrc"],
				subtitle: [1, "subtitle"],
				Ju: [1, "ctaUrl"],
				Iu: [1, "ctaText"]
			},
			ha: 11,
			ia: 6,
			la: [
				["video", ""],
				[1, "hero"],
				[1, "hero__media"],
				[
					"alt",
					"",
					3,
					"src"
				],
				[
					"autoplay",
					"",
					"muted",
					"",
					"loop",
					"",
					"playsinline",
					"",
					3,
					"src"
				],
				[1, "hero__content"],
				[1, "hero__title"],
				[1, "hero__subtitle"],
				[1, "hero__ctas"],
				[
					1,
					"hero__cta",
					"btn",
					"btn--primary",
					"btn--large",
					3,
					"href"
				],
				[
					1,
					"hero__cta",
					"btn",
					3,
					"href",
					"btn--primary",
					"btn--secondary"
				],
				[
					1,
					"hero__cta",
					"btn",
					3,
					"href"
				],
				[3, "iconName"],
				[
					"width",
					"16",
					"height",
					"16",
					"viewBox",
					"0 0 16 16",
					"fill",
					"none",
					"xmlns",
					"http://www.w3.org/2000/svg",
					1,
					"btn__icon-svg"
				],
				[
					"d",
					"M7.91602 15.8332C7.83268 15.8332 7.75629 15.8054 7.68685 15.7498C7.63129 15.6943 7.58963 15.6248 7.56185 15.5415C7.33963 14.6665 6.99935 13.8401 6.54102 13.0623C6.09657 12.2846 5.5549 11.5693 4.91602 10.9165C4.26324 10.2776 3.54796 9.73595 2.77018 9.2915C1.9924 8.83317 1.15907 8.49289 0.270182 8.27067C0.200738 8.24289 0.138238 8.20123 0.0826825 8.14567C0.0271269 8.07623 -0.000650942 7.99984 -0.000650942 7.9165C-0.000650942 7.83317 0.0271269 7.76373 0.0826825 7.70817C0.138238 7.63873 0.200738 7.59011 0.270182 7.56234C1.15907 7.34011 1.9924 7.00678 2.77018 6.56234C3.54796 6.104 4.26324 5.55539 4.91602 4.9165C5.5549 4.26373 6.09657 3.54845 6.54102 2.77067C6.99935 1.99289 7.33963 1.15956 7.56185 0.270669C7.58963 0.201225 7.63129 0.138725 7.68685 0.0831697C7.75629 0.0276142 7.83268 -0.000163555 7.91602 -0.000163555C7.99935 -0.000163555 8.06879 0.0276142 8.12435 0.0831697C8.19379 0.138725 8.2424 0.201225 8.27018 0.270669C8.4924 1.15956 8.82574 1.99289 9.27018 2.77067C9.72852 3.54845 10.2771 4.26373 10.916 4.9165C11.5688 5.55539 12.2841 6.104 13.0619 6.56234C13.8396 7.00678 14.673 7.34011 15.5619 7.56234C15.6313 7.59011 15.6938 7.63873 15.7493 7.70817C15.8049 7.76373 15.8327 7.83317 15.8327 7.9165C15.8327 7.99984 15.8049 8.07623 15.7493 8.14567C15.6938 8.20123 15.6313 8.24289 15.5619 8.27067C14.673 8.49289 13.8396 8.83317 13.0619 9.2915C12.2841 9.73595 11.5688 10.2776 10.916 10.9165C10.2771 11.5693 9.72852 12.2846 9.27018 13.0623C8.82574 13.8401 8.4924 14.6665 8.27018 15.5415C8.2424 15.6248 8.19379 15.6943 8.12435 15.7498C8.06879 15.8054 7.99935 15.8332 7.91602 15.8332Z",
					"fill",
					"#121317"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "section", 1)(1, "div", 2), _.B(2, Gte, 1, 1, "img", 3)(3, Hte, 2, 1, "video", 4), _.H(), _.F(4, "div", 5), _.B(5, Ite, 2, 1, "h1", 6), _.F(6, "p", 7), _.R(7), _.H(), _.F(8, "div", 8), _.B(9, Mte, 2, 0)(10, Nte, 4, 2, "a", 9), _.H()()();
				}
				if (a & 2) {
					_.P("hero--centered", b.layout() === "centered"), _.y(2), _.C(b.mediaType() === "image" ? 2 : b.mediaType() === "video" ? 3 : -1), _.y(3), _.C(b.title() ? 5 : -1), _.y(2), _.S(" ", b.subtitle(), " "), _.y(2), _.C(b.YA() && b.YA().length > 0 ? 9 : 10);
				}
			},
			dependencies: [_.dz],
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*[_ngcontent-%COMP%], [_ngcontent-%COMP%]:after, [_ngcontent-%COMP%]:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}[_nghost-%COMP%]{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff;background-color:#121317}img[_ngcontent-%COMP%], video[_ngcontent-%COMP%]{display:block;max-width:100%;height:auto}a[_ngcontent-%COMP%]{color:inherit;text-decoration:none}button[_ngcontent-%COMP%]{font-family:inherit;cursor:pointer;border:none;background:none}ol[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]{list-style:none}input[_ngcontent-%COMP%], textarea[_ngcontent-%COMP%]{font-family:inherit;border:none;outline:none;background:none}h1[_ngcontent-%COMP%], h2[_ngcontent-%COMP%], h3[_ngcontent-%COMP%], h4[_ngcontent-%COMP%], h5[_ngcontent-%COMP%], h6[_ngcontent-%COMP%]{font-weight:400;line-height:1.2}.text-muted[_ngcontent-%COMP%]{color:#b2bbc5}.text-accent[_ngcontent-%COMP%]{color:#2e96ff}.material-symbols-outlined[_ngcontent-%COMP%]{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}.btn[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;font-family:Google Sans Flex,sans-serif;font-weight:500;font-size:14.5px;line-height:21px;letter-spacing:.11px;border-radius:12px;padding:8px 16px;-webkit-transition:background-color .2s ease;transition:background-color .2s ease;white-space:nowrap;cursor:pointer;text-decoration:none}.btn--primary[_ngcontent-%COMP%]{background-color:#fff;color:#0e0f12}.btn--primary[_ngcontent-%COMP%]:hover{background-color:#e6eaf0}.btn--primary[_ngcontent-%COMP%]:active{background-color:#cdd4dc}.btn--primary.is-disabled[_ngcontent-%COMP%], .btn--primary[_ngcontent-%COMP%]:disabled{background-color:#18191d;color:#45474d;cursor:default}.btn--large[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;padding:14px 24px 14px 20px}.btn--large[_ngcontent-%COMP%]   .btn__icon[_ngcontent-%COMP%]{font-size:20px}.btn__icon[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-family:Material Symbols Outlined;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;font-size:18px;line-height:1;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.btn__icon-svg[_ngcontent-%COMP%]{width:14px;height:14px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.hero[_ngcontent-%COMP%]{position:relative;width:100%;height:100vh;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end;padding-bottom:60px;overflow:hidden;background:#000}@media (min-width:1024px){.hero[_ngcontent-%COMP%]{padding-bottom:120px}}.hero--centered[_ngcontent-%COMP%]{-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center}.hero[_ngcontent-%COMP%]:after{content:\"\";position:absolute;bottom:0;left:0;width:100%;height:278px;background:-webkit-gradient(linear,left bottom,left top,from(#121317),to(rgba(18,19,23,0)));background:-webkit-linear-gradient(bottom,#121317,rgba(18,19,23,0));background:linear-gradient(0deg,#121317,rgba(18,19,23,0));z-index:1;pointer-events:none}.hero__media[_ngcontent-%COMP%]{position:absolute;inset:0;margin:0 auto;width:100%;max-width:1500px;z-index:0}.hero__media[_ngcontent-%COMP%]:before{content:\"\";position:absolute;inset:0;z-index:1;pointer-events:none;background:-webkit-gradient(linear,left top,right top,from(#000),to(transparent)) 0 0/10% 100% no-repeat,-webkit-gradient(linear,right top,left top,from(#000),to(transparent)) 100% 0/10% 100% no-repeat,-webkit-gradient(linear,left top,left bottom,from(#000),to(transparent)) 0 0/100% 10% no-repeat;background:-webkit-linear-gradient(left,#000,transparent) 0 0/10% 100% no-repeat,-webkit-linear-gradient(right,#000,transparent) 100% 0/10% 100% no-repeat,-webkit-linear-gradient(top,#000,transparent) 0 0/100% 10% no-repeat;background:linear-gradient(90deg,#000,transparent) 0 0/10% 100% no-repeat,linear-gradient(270deg,#000,transparent) 100% 0/10% 100% no-repeat,linear-gradient(180deg,#000,transparent) 0 0/100% 10% no-repeat}.hero__media[_ngcontent-%COMP%]   img[_ngcontent-%COMP%], .hero__media[_ngcontent-%COMP%]   video[_ngcontent-%COMP%]{width:100%;height:100%;object-fit:contain}.hero__content[_ngcontent-%COMP%]{position:relative;z-index:2;text-align:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.hero__title[_ngcontent-%COMP%]{font-size:32px;font-weight:500;line-height:1.2;color:#fff;margin-bottom:16px}.hero--centered[_ngcontent-%COMP%]   .hero__title[_ngcontent-%COMP%]{font-size:48px;margin-bottom:24px}@media (min-width:1024px){.hero--centered[_ngcontent-%COMP%]   .hero__title[_ngcontent-%COMP%]{font-size:124px;margin-bottom:50px}}.hero__subtitle[_ngcontent-%COMP%]{font-size:14.5px;line-height:21px;letter-spacing:.11px;color:#cdd4dc;max-width:300px;margin-bottom:24px}@media (min-width:1024px){.hero__subtitle[_ngcontent-%COMP%]{font-size:17.5px;line-height:25.375px;letter-spacing:.1925px;max-width:418px;margin-bottom:32px}}.hero__ctas[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;margin-top:8px;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap}.hero__ctas[_ngcontent-%COMP%]   .btn--secondary[_ngcontent-%COMP%]{background:rgba(183,191,217,.2);border-color:rgba(183,191,217,.2)}.hero__ctas[_ngcontent-%COMP%]   .btn--secondary[_ngcontent-%COMP%]:hover{background:rgba(183,191,217,.4);border-color:rgba(183,191,217,.4)}.hero__cta[_ngcontent-%COMP%]{margin-top:0}"]
		});
		Yte = function(a) {
			if (a & 1) {
				_.F(0, "h2", 1), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.U(a.headline());
			}
		};
		Zte = function(a) {
			if (a & 1) {
				_.F(0, "div", 6), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K().V, _.y(), _.U(a.description);
			}
		};
		$te = function(a) {
			if (a & 1) {
				_.F(0, "div", 7), _.R(1), _.I(2, "span", 9), _.H();
			}
			if (a & 2) {
				a = _.K().V;
				let b = _.K();
				_.y();
				_.S(" ", a.Sl, " ");
				_.y();
				_.E("iconName", b.S.gh);
			}
		};
		aue = function(a) {
			if (a & 1) {
				_.I(0, "span", 8);
			}
			if (a & 2) {
				a = _.K(2), _.E("iconName", a.S.gh);
			}
		};
		bue = function(a, b) {
			if (a & 1) {
				_.F(0, "a", 4)(1, "div", 5), _.R(2), _.H(), _.B(3, Zte, 2, 1, "div", 6), _.B(4, $te, 3, 2, "div", 7), _.B(5, aue, 1, 1, "span", 8), _.H();
			}
			if (a & 2) {
				a = b.V, _.P("resources__card--compact", !a.description), _.E("href", a.url, _.rg), _.y(2), _.U(a.title), _.y(), _.C(a.description ? 3 : -1), _.y(), _.C(a.description && a.Sl ? 4 : -1), _.y(), _.C(a.description ? -1 : 5);
			}
		};
		_.x8 = class {
			constructor() {
				this.S = _.Dk;
				this.headline = _.V("");
				this.items = _.Li([]);
			}
		};
		_.x8.J = function(a) {
			return new (a || _.x8)();
		};
		_.x8.ka = _.u({
			type: _.x8,
			da: [["ms-marketing-resources"]],
			inputs: {
				headline: [1, "headline"],
				items: [1, "items"]
			},
			ha: 5,
			ia: 1,
			la: [
				[1, "resources"],
				[1, "resources__title"],
				[1, "resources__grid"],
				[
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"resources__card",
					3,
					"resources__card--compact",
					"href"
				],
				[
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"resources__card",
					3,
					"href"
				],
				[1, "resources__card-title"],
				[1, "resources__card-desc"],
				[1, "resources__card-link"],
				[
					1,
					"resources__card-icon-absolute",
					3,
					"iconName"
				],
				[3, "iconName"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "section", 0), _.B(1, Yte, 2, 1, "h2", 1), _.F(2, "div", 2), _.Ah(3, bue, 6, 7, "a", 3, _.yh), _.H()();
				}
				if (a & 2) {
					_.y(), _.C(b.headline() ? 1 : -1), _.y(2), _.Bh(b.items());
				}
			},
			dependencies: [_.tz, _.dz],
			styles: [":root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}.resources{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;width:100%;max-width:1540px;margin-left:auto;margin-right:auto;gap:20px;padding-left:20px;padding-right:20px;padding-top:40px;padding-bottom:40px}@media (min-width:1024px){.resources{max-width:1580px;padding-left:40px;padding-right:40px}}@media (min-width:1024px){.resources{padding-top:80px;padding-bottom:80px}}.resources__title{font-family:Google Sans Flex,sans-serif;font-size:45px;line-height:1.1;text-align:left;color:#fff;margin-bottom:36px;font-weight:400}.resources__grid{display:grid;grid-template-columns:1fr;gap:20px 20px;width:100%}@media (min-width:768px){.resources__grid{grid-template-columns:repeat(2,1fr)}}@media (min-width:1024px){.resources__grid{grid-template-columns:repeat(4,1fr)}}.resources__card{position:relative;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;background-color:#212226;border-radius:16px;padding:24px;text-decoration:none;color:inherit;-webkit-transition:opacity .2s ease;transition:opacity .2s ease;height:100%}.resources__card:hover{opacity:.8}.resources__card--compact{padding-bottom:50px;padding-right:50px;min-block-size:112px}.resources__card--compact .resources__card-title{max-inline-size:170px}.resources__card-title{font-family:Google Sans Flex,sans-serif;font-size:20px;line-height:25.4px;color:#fff;margin-bottom:8px}.resources__card-desc{font-family:Google Sans Flex,sans-serif;font-size:14.5px;line-height:21px;color:#b2bbc5;margin-bottom:24px}.resources__card-link{font-family:Google Sans Flex,sans-serif;font-size:14.5px;line-height:21px;color:#fff;font-weight:500;margin-top:auto;display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-align-self:flex-start;-ms-flex-item-align:start;align-self:flex-start;gap:4px}.resources__card-link .material-symbols-outlined{font-size:20px}.resources__card-icon-absolute{position:absolute;top:24px;right:24px;font-size:20px;color:#fff}"],
			Ab: 2
		});
		var hue;
		var lue;
		eue = function(a) {
			if (a & 1) {
				_.Dh(0, "h2", 1), _.R(1), _.Eh();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.U(a.headline());
			}
		};
		fue = function(a) {
			if (a & 1) {
				_.Dh(0, "p", 11), _.R(1), _.Eh();
			}
			if (a & 2) {
				a = _.K().V, _.y(), _.U(a.author);
			}
		};
		gue = function(a, b) {
			if (a & 1) {
				_.Dh(0, "div", 9)(1, "p", 10), _.R(2), _.Eh(), _.B(3, fue, 2, 1, "p", 11), _.Eh();
			}
			if (a & 2) {
				a = b.V;
				b = b.jb;
				let c = _.K(2);
				_.P("quote__slide--active", b === c.lg);
				_.y(2);
				_.U(a.text);
				_.y();
				_.C(a.author ? 3 : -1);
			}
		};
		iue = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.Dh(0, "button", 12);
				_.Wh("click", function() {
					var d = _.q(c).jb;
					var e = _.K(2);
					e.iY = true;
					e.lg = d + 1;
					if (_.Uk(e.Jsa) && e.quotes().length > 1) {
						hue(e);
					}
					return _.t();
				});
				_.Eh();
			}
			if (a & 2) {
				a = b.jb, b = _.K(2), _.P("quote__bullet--active", a === b.Zvb);
			}
		};
		jue = function(a) {
			if (a & 1) {
				let b = _.n();
				_.Dh(0, "div", 4);
				_.Wh("mousedown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.aF(c));
				})("mousemove", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.eN(c));
				})("mouseup", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.sx());
				})("mouseleave", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.sx());
				})("touchstart", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.aF(c));
				})("touchmove", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.eN(c));
				})("touchend", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.sx());
				});
				_.Dh(1, "div", 5);
				_.Ah(2, gue, 4, 4, "div", 6, _.yh);
				_.Eh();
				_.Dh(4, "div", 7);
				_.Ah(5, iue, 1, 2, "button", 8, _.yh);
				_.Eh()();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.pi("--active-index", a.lg), _.P("quote__track--transition", a.iY), _.y(), _.Bh(a.ezb), _.y(3), _.Bh(a.quotes());
			}
		};
		kue = function(a) {
			if (a & 1) {
				_.Dh(0, "div", 3)(1, "p", 10), _.R(2), _.Eh()();
			}
			if (a & 2) {
				a = _.K(), _.y(2), _.U(a.text());
			}
		};
		hue = function(a) {
			lue(a);
			a.timer = setInterval(() => {
				a.next();
				a.F.lb();
			}, 5e3);
		};
		lue = function(a) {
			if (a.timer) {
				clearInterval(a.timer), a.timer = null;
			}
		};
		mue = function(a) {
			return typeof window !== "undefined" && TouchEvent && a instanceof TouchEvent ? a.changedTouches ? a.changedTouches[0].pageX : a.touches[0].pageX : a.pageX;
		};
		_.y8 = class {
			constructor() {
				this.headline = _.V("");
				this.text = _.V("");
				this.quotes = _.Li([]);
				this.lg = 1;
				this.iY = true;
				this.Sc = false;
				this.H = this.I = 0;
				this.A = this.timer = null;
				this.F = _.m(_.Hu);
				this.ub = _.m(_.ag);
				this.Jsa = _.m(_.Au);
			}
			get ezb() {
				var a = this.quotes();
				return a.length <= 1 ? [...a] : [
					a[a.length - 1],
					...a,
					a[0]
				];
			}
			get Zvb() {
				var a = this.quotes().length;
				return a <= 1 ? 0 : this.lg === 0 ? a - 1 : this.lg === a + 1 ? 0 : this.lg - 1;
			}
			ib() {
				if (this.quotes().length > 1) {
					this.lg = 1, _.Uk(this.Jsa) && hue(this);
				}
				this.ub.Hc(() => {
					lue(this);
					if (this.A) {
						clearTimeout(this.A);
					}
				});
			}
			next() {
				var a = this.quotes();
				if (!(a.length <= 1)) {
					this.iY = true, this.lg++, this.lg === a.length + 1 && (this.A && clearTimeout(this.A), this.A = setTimeout(() => {
						this.iY = false;
						this.lg = 1;
						this.F.lb();
					}, 700));
				}
			}
			prev() {
				var a = this.quotes();
				if (!(a.length <= 1)) {
					this.iY = true, this.lg--, this.lg === 0 && (this.A && clearTimeout(this.A), this.A = setTimeout(() => {
						this.iY = false;
						this.lg = a.length;
						this.F.lb();
					}, 700));
				}
			}
			aF(a) {
				this.Sc = true;
				this.H = this.I = mue(a);
				lue(this);
				if (this.A) {
					clearTimeout(this.A), this.A = null;
				}
			}
			eN(a) {
				if (this.Sc) {
					this.H = mue(a), a.preventDefault();
				}
			}
			sx() {
				if (this.Sc) {
					this.Sc = false;
					var a = this.H - this.I;
					if (a > 50) {
						this.prev(), this.F.lb();
					} else {
						if (a < -50) {
							this.next(), this.F.lb();
						}
					}
					if (_.Uk(this.Jsa) && this.quotes().length > 1) {
						hue(this);
					}
				}
			}
		};
		_.y8.J = function(a) {
			return new (a || _.y8)();
		};
		_.y8.ka = _.u({
			type: _.y8,
			da: [["ms-marketing-quote"]],
			inputs: {
				headline: [1, "headline"],
				text: [1, "text"],
				quotes: [1, "quotes"]
			},
			ha: 4,
			ia: 2,
			la: [
				[1, "quote-section"],
				[1, "quote__headline"],
				[1, "quote__carousel"],
				[1, "quote"],
				[
					1,
					"quote__carousel",
					3,
					"mousedown",
					"mousemove",
					"mouseup",
					"mouseleave",
					"touchstart",
					"touchmove",
					"touchend"
				],
				[1, "quote__track"],
				[
					1,
					"quote__slide",
					3,
					"quote__slide--active"
				],
				[1, "quote__pagination"],
				[
					"aria-label",
					"Go to slide",
					1,
					"quote__bullet",
					3,
					"quote__bullet--active"
				],
				[1, "quote__slide"],
				[1, "quote__text"],
				[1, "quote__author"],
				[
					"aria-label",
					"Go to slide",
					1,
					"quote__bullet",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Dh(0, "section", 0), _.B(1, eue, 2, 1, "h2", 1), _.B(2, jue, 7, 4, "div", 2)(3, kue, 3, 1, "div", 3), _.Eh();
				}
				if (a & 2) {
					_.y(), _.C(b.headline() ? 1 : -1), _.y(), _.C(b.quotes().length > 0 ? 2 : 3);
				}
			},
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}.quote[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;text-align:center;padding:0 20px}@media (min-width:1024px){.quote[_ngcontent-%COMP%]{padding:0 40px}}.quote__text[_ngcontent-%COMP%]{font-size:28px;line-height:29.7px;letter-spacing:-.28px;font-weight:400;font-family:Google Sans Flex,sans-serif;color:#fff;max-width:900px;-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none}@media (min-width:1024px){.quote__text[_ngcontent-%COMP%]{font-size:42px;line-height:43.7px;letter-spacing:-.84px;font-weight:450}}.quote__carousel[_ngcontent-%COMP%]{position:relative;width:100%;overflow:hidden;padding:0;cursor:-webkit-grab;cursor:-moz-grab;cursor:grab}.quote__carousel[_ngcontent-%COMP%]:active{cursor:-webkit-grabbing;cursor:-moz-grabbing;cursor:grabbing}.quote__track[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-transform:translateX(calc(var(--active-index, 1)*-100%));transform:translateX(calc(var(--active-index, 1)*-100%))}.quote__track--transition[_ngcontent-%COMP%]{-webkit-transition:-webkit-transform .7s cubic-bezier(.455,.03,.515,.955);transition:-webkit-transform .7s cubic-bezier(.455,.03,.515,.955);transition:transform .7s cubic-bezier(.455,.03,.515,.955);transition:transform .7s cubic-bezier(.455,.03,.515,.955),-webkit-transform .7s cubic-bezier(.455,.03,.515,.955)}.quote__track--transition[_ngcontent-%COMP%]   .quote__slide[_ngcontent-%COMP%]{-webkit-transition:opacity .7s cubic-bezier(.455,.03,.515,.955);transition:opacity .7s cubic-bezier(.455,.03,.515,.955)}.quote__slide[_ngcontent-%COMP%]{-webkit-box-flex:0;-webkit-flex:0 0 100%;-moz-box-flex:0;-ms-flex:0 0 100%;flex:0 0 100%;width:100%;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;text-align:center;padding:0 20px;opacity:0}.quote__slide--active[_ngcontent-%COMP%]{opacity:1}.quote__author[_ngcontent-%COMP%]{font-size:16px;line-height:1.5;color:#b2bbc5;margin-top:20px;-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none}.quote__pagination[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;gap:8px;margin-top:40px}.quote__bullet[_ngcontent-%COMP%]{width:8px;height:8px;border-radius:50%;background:hsla(0,0%,100%,.2);border:none;padding:0;cursor:pointer;-webkit-transition:background .3s ease;transition:background .3s ease}.quote__bullet--active[_ngcontent-%COMP%]{background:#fff}.quote-section[_ngcontent-%COMP%]{padding:80px 0;background:#121317}@media (min-width:1024px){.quote-section[_ngcontent-%COMP%]{padding:180px 0}}.quote__headline[_ngcontent-%COMP%]{font-size:28px;line-height:1.2;letter-spacing:-.5px;font-weight:500;color:#fff;text-align:center;margin-bottom:40px}@media (min-width:1024px){.quote__headline[_ngcontent-%COMP%]{font-size:28px;line-height:29.7px;letter-spacing:-.28px;font-weight:400;margin-bottom:80px}}"]
		});
		_.hr("smxHue");
		_.Tr = class {
			constructor() {
				this.jZ = "Managed agents in Gemini API";
				this.It = "video";
				this.Eq = "https://www.gstatic.com/aistudio-static/editorial/managed-agents/managed-agents_hero.mp4";
				this.Jt = "Fully hosted agents powered by Antigravity and Gemini";
				this.AB = [{
					text: "Get started",
					url: "https://aistudio.google.com/agents",
					variant: "primary",
					icon: "login"
				}, {
					text: "Docs",
					url: "https://ai.google.dev/gemini-api/docs/agents",
					variant: "primary",
					icon: "docs"
				}];
				this.blocks = [
					{
						Vk: "font",
						icon: "api",
						title: "One API call to fully managed agent",
						description: "Skip the infrastructure setup. A single API call gives your agent instructions, tools, and a remote Linux environment to reason and execute code.",
						video: "https://www.gstatic.com/aistudio-static/editorial/managed-agents/managed-agents_01.mp4"
					},
					{
						Vk: "font",
						icon: "lock_open_circle",
						title: "Persistent, stateful environments",
						description: "Set up your environment once. Your Git repos, packages, inline assets, and GCS objects persist across sessions.",
						video: "https://www.gstatic.com/aistudio-static/editorial/managed-agents/managed-agents_02.mp4"
					},
					{
						Vk: "font",
						icon: "antigravity",
						title: "Antigravity agent that powers Google’s own",
						description: "Build on the same production-grade agentic harness, co-optimized with Gemini, that powers many of Google’s own agents.",
						video: "https://www.gstatic.com/aistudio/managed-agents/agents_03.mp4"
					}
				];
				this.gTb = "What developers are building";
				this.hTb = [
					{
						author: "Givi Beridze, Klipy",
						text: "“The API’s intelligence allows us to handle complex workflows with remarkable precision, yet it remains intuitive to implement. Balancing high-level reasoning with true ease of use enables our team to deploy advanced automation faster than ever.”"
					},
					{
						author: "Zohaib Ahmed, Resemble AI",
						text: "“Prompt to autonomous agent in a single API call feels magical. We had two real agents running on Google’s stack the day we got access. The harness owning tools, sandbox, and state across turns makes the orchestration layer disappear. We can focus on the agent instead of the environment running them.”"
					},
					{
						author: "René Sultan, Ramp",
						text: "“The real shift with Managed Agents is that the agent runtime moves into the platform. With the sandbox, infrastructure, and execution loop managed for you, developers can focus on productizing the agent’s domain-specific behavior and iterating at a different pace.”"
					}
				];
				this.qxa = [{
					id: "bash",
					label: "Shell"
				}];
				this.BWa = "Get started";
				this.GWa = "Quickstart";
				this.AWa = "Set up your environment and make your first API request in minutes";
				this.CWa = "Get API Key";
				this.DWa = "https://aistudio.google.com/api-keys";
				this.EWa = "View quickstart";
				this.FWa = "https://ai.google.dev/gemini-api/docs/managed-agents-quickstart";
				this.pxa = { bash: {
					code: "agent = client.agents.create(\n    name=\"data-analyst\",\n    base_agent=\"antigravity-preview-05-2026\",\n    base_environment={\n        \"type\": \"remote\",\n        \"sources\": [\n            {\"type\": \"inline\", \"target\": \".agents/agents.md\", \"content\": \"You are a data analyst agent...\"},\n            {\"type\": \"inline\", \"target\": \".agents/slide-maker/SKILL.md\", \"content\": \"---name: slide-maker ...--- ...\"}\n        ]\n    }\n)\n",
					lang: "bash"
				} };
				this.vIa = "Integrations";
				this.wIa = [
					{
						title: "Agno",
						url: "https://docs.agno.com/agent-os/multi-framework/antigravity",
						description: ""
					},
					{
						title: "Eigent",
						url: "https://www.eigent.ai/use-cases/eigent-gemini-managed-agents",
						description: ""
					},
					{
						title: "LiteLLM",
						url: "https://docs.litellm.ai/docs/managed_agents",
						description: ""
					},
					{
						title: "LlamaIndex",
						url: "https://github.com/run-llama/antigravity-demo",
						description: ""
					},
					{
						title: "Vercel",
						url: "https://ai-sdk.dev/providers/ai-sdk-providers/google-generative-ai#agent-presets",
						description: ""
					}
				];
			}
		};
		_.Tr.J = function(a) {
			return new (a || _.Tr)();
		};
		_.Tr.ka = _.u({
			type: _.Tr,
			da: [["ms-marketing-managed-agents-screen"]],
			eb: [
				1,
				"marketing-v2-theme",
				"dark-theme"
			],
			ha: 7,
			ia: 11,
			la: [
				[
					"title",
					"Managed Agents in Gemini API",
					3,
					"subtitle",
					"mediaType",
					"mediaSrc",
					"ctas"
				],
				[3, "blocks"],
				[
					3,
					"headline",
					"quotes"
				],
				[
					"mainTitle",
					"Get started",
					"title",
					"Quickstart",
					"description",
					"Set up your environment and make your first API request in minutes",
					"primaryCtaText",
					"Get API Key",
					"primaryCtaUrl",
					"https://aistudio.google.com/api-keys",
					"secondaryCtaText",
					"View quickstart",
					"secondaryCtaUrl",
					"https://ai.google.dev/gemini-api/docs/managed-agents-quickstart",
					3,
					"tabs",
					"snippets"
				],
				[
					3,
					"headline",
					"items"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.I(0, "ms-marketing-nav")(1, "ms-marketing-hero", 0)(2, "ms-marketing-blocks", 1)(3, "ms-marketing-quote", 2)(4, "ms-marketing-code-snippet", 3)(5, "ms-marketing-resources", 4)(6, "ms-marketing-footer");
				}
				if (a & 2) {
					_.y(), _.E("subtitle", b.Jt)("mediaType", b.It)("mediaSrc", b.Eq)("ctas", b.AB), _.y(), _.E("blocks", b.blocks), _.y(), _.E("headline", b.gTb)("quotes", b.hTb), _.y(), _.E("tabs", b.qxa)("snippets", b.pxa), _.y(), _.E("headline", b.vIa)("items", b.wIa);
				}
			},
			dependencies: [
				_.i8,
				_.u8,
				_.r8,
				_.y8,
				_.t8,
				_.x8,
				_.h8
			],
			styles: ["[_ngcontent-%COMP%]:root{--ms-marketing-color-bg:#121317;--ms-marketing-color-text-primary:#fff;--ms-marketing-color-text-muted:#b2bbc5;--ms-marketing-color-text-nav:#d4d4d4;--ms-marketing-color-accent-blue:#2e96ff;--ms-marketing-color-button-bg:#fff;--ms-marketing-color-button-text:#0e0f12;--ms-marketing-color-neutral-90:#1e1f23;--ms-marketing-color-bg-container:#1a1b1f}*[_ngcontent-%COMP%], [_ngcontent-%COMP%]:after, [_ngcontent-%COMP%]:before{-moz-box-sizing:border-box;box-sizing:border-box;margin:0;padding:0}[_nghost-%COMP%]{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;font-family:Google Sans Flex,sans-serif;font-weight:400;font-size:16px;line-height:1.5;color:#fff}img[_ngcontent-%COMP%], video[_ngcontent-%COMP%]{display:block;max-width:100%;height:auto}a[_ngcontent-%COMP%]{color:inherit;text-decoration:none}button[_ngcontent-%COMP%]{font-family:inherit;cursor:pointer;border:none;background:none}ol[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]{list-style:none}input[_ngcontent-%COMP%], textarea[_ngcontent-%COMP%]{font-family:inherit;border:none;outline:none;background:none}h1[_ngcontent-%COMP%], h2[_ngcontent-%COMP%], h3[_ngcontent-%COMP%], h4[_ngcontent-%COMP%], h5[_ngcontent-%COMP%], h6[_ngcontent-%COMP%]{font-weight:400;line-height:1.2}.text-muted[_ngcontent-%COMP%]{color:#b2bbc5}.text-accent[_ngcontent-%COMP%]{color:#2e96ff}.material-symbols-outlined[_ngcontent-%COMP%]{font-family:Google Symbols;font-weight:400;font-style:normal;font-size:24px;line-height:1;letter-spacing:normal;text-transform:none;display:inline-block;white-space:nowrap;word-wrap:normal;direction:ltr;-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;text-rendering:optimizeLegibility;-webkit-font-feature-settings:\"liga\";-moz-font-feature-settings:\"liga\";font-feature-settings:\"liga\";font-variation-settings:\"FILL\" 0,\"wght\" 300,\"GRAD\" 0,\"opsz\" 30}[_nghost-%COMP%]{display:block;height:100%;overflow:hidden auto;background-color:#121317}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

