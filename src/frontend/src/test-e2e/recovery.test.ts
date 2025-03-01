import { FLOWS } from "./flows";
import { addVirtualAuthenticator, runInBrowser } from "./util";
import { MainView, RecoverView, WelcomeView } from "./views";

import { DEVICE_NAME1, II_URL, RECOVERY_PHRASE_NAME } from "./constants";

test("Recover access, after registration", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    const userNumber = await FLOWS.registerNewIdentityWelcomeView(
      DEVICE_NAME1,
      browser
    );
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();
    await mainView.logout();

    const welcomeView = new WelcomeView(browser);
    await welcomeView.recover();
    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForDisplay();
    await recoveryView.enterIdentityAnchor(userNumber);
    await recoveryView.continue();
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase(seedPhrase);
    await recoveryView.enterSeedPhraseContinue();
    await recoveryView.skipDeviceEnrollment();
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
  });
}, 300_000);

test("Reset unprotected recovery phrase", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    await FLOWS.registerNewIdentityWelcomeView(DEVICE_NAME1, browser);
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();

    await mainView.waitForDeviceDisplay(RECOVERY_PHRASE_NAME);

    // Ensure the settings dropdown is in view
    await browser.execute("window.scrollTo(0, document.body.scrollHeight)");
    await mainView.reset(RECOVERY_PHRASE_NAME);
    await browser.acceptAlert();
    const _seedPhrase = await FLOWS.readSeedPhrase(browser);
    await mainView.waitForDisplay();
  });
}, 300_000);

test("Recover access, after reset", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    const userNumber = await FLOWS.registerNewIdentityWelcomeView(
      DEVICE_NAME1,
      browser
    );
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);

    // Create a phrase
    const _seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();

    // Reset the phrase
    // Ensure the settings dropdown is in view
    await browser.execute("window.scrollTo(0, document.body.scrollHeight)");
    await mainView.reset(RECOVERY_PHRASE_NAME);
    await browser.acceptAlert();
    const seedPhrase = await FLOWS.readSeedPhrase(browser);
    await mainView.waitForDisplay();

    // Logout
    await mainView.logout();

    // Recover with new phrase
    const welcomeView = new WelcomeView(browser);
    await welcomeView.recover();
    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForDisplay();
    await recoveryView.enterIdentityAnchor(userNumber);
    await recoveryView.continue();
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase(seedPhrase);
    await recoveryView.enterSeedPhraseContinue();
    await recoveryView.skipDeviceEnrollment();
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
  });
}, 300_000);

test("Canceling reset keeps old phrase", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    const userNumber = await FLOWS.registerNewIdentityWelcomeView(
      DEVICE_NAME1,
      browser
    );
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);

    // Create a phrase
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();

    // Reset the phrase
    // Ensure the settings dropdown is in view
    await browser.execute("window.scrollTo(0, document.body.scrollHeight)");
    await mainView.reset(RECOVERY_PHRASE_NAME);
    await browser.acceptAlert();
    // Instead of reading the new seed phrase, cancel the flow
    await browser.$(`button[data-action='cancel']`).click();
    await mainView.waitForDisplay();

    // Logout
    await mainView.logout();

    // Recover with old phrase
    const welcomeView = new WelcomeView(browser);
    await welcomeView.recover();
    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForDisplay();
    await recoveryView.enterIdentityAnchor(userNumber);
    await recoveryView.continue();
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase(seedPhrase);
    await recoveryView.enterSeedPhraseContinue();
    await recoveryView.skipDeviceEnrollment();
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
  });
}, 300_000);

test("Reset unprotected recovery phrase, when authenticated with phrase", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    const userNumber = await FLOWS.registerNewIdentityWelcomeView(
      DEVICE_NAME1,
      browser
    );
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();
    await mainView.logout();

    const welcomeView = new WelcomeView(browser);
    await welcomeView.recover();
    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForDisplay();
    await recoveryView.enterIdentityAnchor(userNumber);
    await recoveryView.continue();
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase(seedPhrase);
    await recoveryView.enterSeedPhraseContinue();
    await recoveryView.skipDeviceEnrollment();
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);

    // Ensure the settings dropdown is in view
    await browser.execute("window.scrollTo(0, document.body.scrollHeight)");
    await mainView.reset(RECOVERY_PHRASE_NAME);
    await browser.acceptAlert();
    const _seedPhrase = await FLOWS.readSeedPhrase(browser);
    await mainView.waitForDisplay();
  });
}, 300_000);

test("Make recovery protected", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    await FLOWS.registerNewIdentityWelcomeView(DEVICE_NAME1, browser);
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();
    await mainView.assertDeviceUnprotected(RECOVERY_PHRASE_NAME);
    await mainView.protect(RECOVERY_PHRASE_NAME, seedPhrase);
    await mainView.assertDeviceProtected(RECOVERY_PHRASE_NAME);
  });
}, 300_000);

test("Make recovery unprotected", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    await FLOWS.registerNewIdentityWelcomeView(DEVICE_NAME1, browser);
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();
    await mainView.assertDeviceUnprotected(RECOVERY_PHRASE_NAME);
    await mainView.protect(RECOVERY_PHRASE_NAME, seedPhrase);
    await mainView.assertDeviceProtected(RECOVERY_PHRASE_NAME);

    await mainView.unprotect(RECOVERY_PHRASE_NAME, seedPhrase);
    await mainView.assertDeviceUnprotected(RECOVERY_PHRASE_NAME);
  });
}, 300_000);

test("Reset protected recovery phrase", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    await FLOWS.registerNewIdentityWelcomeView(DEVICE_NAME1, browser);
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();

    await mainView.protect(RECOVERY_PHRASE_NAME, seedPhrase);

    await mainView.waitForDisplay();
    await mainView.reset(RECOVERY_PHRASE_NAME);
    await browser.acceptAlert();

    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase(seedPhrase);
    await recoveryView.enterSeedPhraseContinue();

    const _seedPhrase = await FLOWS.readSeedPhrase(browser);

    await mainView.waitForDisplay();
  });
}, 300_000);

test("Reset protected recovery phrase, confirm with empty seed phrase", async () => {
  await runInBrowser(async (browser: WebdriverIO.Browser) => {
    await addVirtualAuthenticator(browser);
    await browser.url(II_URL);
    await FLOWS.registerNewIdentityWelcomeView(DEVICE_NAME1, browser);
    const mainView = new MainView(browser);
    await mainView.waitForDeviceDisplay(DEVICE_NAME1);
    const seedPhrase = await FLOWS.addRecoveryMechanismSeedPhrase(browser);
    await mainView.waitForDisplay();

    await mainView.protect(RECOVERY_PHRASE_NAME, seedPhrase);
    await mainView.waitForDisplay();
    await mainView.reset(RECOVERY_PHRASE_NAME);

    await browser.acceptAlert();

    const recoveryView = new RecoverView(browser);
    await recoveryView.waitForSeedInputDisplay();
    await recoveryView.enterSeedPhrase("");
    await recoveryView.enterSeedPhraseContinue();
    await recoveryView.waitForInvalidSeedPhraseDisplay();
  });
}, 300_000);
