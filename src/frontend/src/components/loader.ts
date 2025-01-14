import { html, render } from "lit-html";

const loader = () => html` <div id="loader" class="c-loader">
  <img
    class="c-loader__image"
    src=${process.env.BASER_URL ?? "" + "/loader.webp"}
    alt="loading"
  />
</div>`;

const startLoader = () => {
  const container = document.getElementById("loaderContainer") as HTMLElement;
  render(loader(), container);
};

const endLoader = () => {
  const container = document.getElementById("loaderContainer") as HTMLElement;
  render(html``, container);
};

export const withLoader = async <A>(action: () => Promise<A>): Promise<A> => {
  startLoader();
  try {
    return await action();
  } finally {
    endLoader();
  }
};
