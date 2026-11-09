-- What a provider's own model listing says about a model's behaviour.
--
-- Additive only: an older devd ignores these columns and keeps using the
-- model-family table for tools, images and reasoning.
ALTER TABLE model_catalogue ADD COLUMN tools INTEGER;
ALTER TABLE model_catalogue ADD COLUMN vision INTEGER;
ALTER TABLE model_catalogue ADD COLUMN reasoning INTEGER;
