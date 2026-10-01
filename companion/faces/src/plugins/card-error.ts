/** A deterministic card refusal, unless wrapping an unknown layout-engine error. */
export class CardError extends Error {
  override name = "CardError";

  constructor(
    message: string,
    public configuration = true,
  ) {
    super(message);
  }
}
