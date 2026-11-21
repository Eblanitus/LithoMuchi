package dev.litospherity.fabric.mixin;

import dev.litospherity.fabric.LithoWaterFeature;
import dev.litospherity.fabric.StaticRiverMask;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.ticks.LevelChunkTicks;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(ProtoChunk.class)
public abstract class ProtoChunkRiverMixin implements StaticRiverMask.Holder {
    @Unique private StaticRiverMask litospherity$river;

    @Override
    public void litospherity$rememberRiver(StaticRiverMask mask) { litospherity$river = mask; }

    @Inject(method = "unpackFluidTicks", at = @At("RETURN"), cancellable = true)
    private void litospherity$finishRiver(CallbackInfoReturnable<LevelChunkTicks<Fluid>> cir) {
        var mask = litospherity$river;
        litospherity$river = null;
        if (mask != null && !LithoWaterFeature.LIVE_STEPS) {
            cir.setReturnValue(mask.finish((ProtoChunk)(Object)this, cir.getReturnValue()));
        }
    }
}
