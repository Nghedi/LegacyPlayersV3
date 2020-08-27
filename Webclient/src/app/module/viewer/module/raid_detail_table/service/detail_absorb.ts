import {Injectable, OnDestroy} from "@angular/core";
import {BehaviorSubject, Observable, Subscription} from "rxjs";
import {SelectOption} from "../../../../../template/input/select_input/domain_value/select_option";
import {HitType} from "../../../domain_value/hit_type";
import {DetailRow} from "../domain_value/detail_row";
import {InstanceDataService} from "../../../service/instance_data";
import {SpellService} from "../../../service/spell";
import {KnechtUpdates} from "../../../domain_value/knecht_updates";
import {ABSORBING_SPELL_IDS, get_absorb_data_points} from "../../../stdlib/absorb";
import {detail_row_post_processing, fill_details} from "../stdlib/util";

@Injectable({
    providedIn: "root",
})
export class DetailAbsorbService implements OnDestroy {

    private subscription: Subscription;

    private abilities$: BehaviorSubject<Array<SelectOption>> = new BehaviorSubject([]);
    private ability_details$: BehaviorSubject<Array<[number, Array<[HitType, DetailRow]>]>> = new BehaviorSubject([]);

    private initialized: boolean = false;

    private current_mode: boolean = false;

    constructor(
        private instanceDataService: InstanceDataService,
        private spellService: SpellService
    ) {
    }

    ngOnDestroy(): void {
        this.subscription?.unsubscribe();
    }

    get_ability_and_details(mode: boolean): [Observable<Array<SelectOption>>, Observable<Array<[number, Array<[HitType, DetailRow]>]>>] {
        if (!this.initialized) {
            this.current_mode = mode;
            this.initialize();
        } else if (this.current_mode !== mode) {
            this.current_mode = mode;
            this.commit();
        }
        return [this.abilities$.asObservable(), this.ability_details$.asObservable()];
    }

    private initialize(): void {
        this.initialized = true;
        this.subscription = this.instanceDataService.knecht_updates.subscribe(async knecht_update => {
            if (knecht_update.some(elem => [KnechtUpdates.NewData, KnechtUpdates.FilterChanged].includes(elem)))
                this.commit();
        });
        this.commit();
    }

    private async commit(): Promise<void> {
        const abilities = new Map<number, SelectOption>();
        const result = new Map();

        const absorbs = await get_absorb_data_points(this.current_mode, this.instanceDataService);
        for (const [subject_id, [subject, points]] of absorbs) {
            for (const [absorbed_spell_id, timestamp, amount] of points) {
                if (!abilities.has(absorbed_spell_id))
                    abilities.set(absorbed_spell_id, { value: absorbed_spell_id, label_key: this.spellService.get_spell_name(absorbed_spell_id) });
                if (!result.has(absorbed_spell_id))
                    result.set(absorbed_spell_id, new Map());
                const details_map = result.get(absorbed_spell_id);
                fill_details([{
                    school_mask: ABSORBING_SPELL_IDS.get(absorbed_spell_id)[1],
                    amount,
                    mitigation: []
                }], [HitType.Hit], details_map);
            }
        }

        // @ts-ignore
        this.abilities$.next([...abilities.values()]);
        this.ability_details$.next(detail_row_post_processing(result));
    }
}
